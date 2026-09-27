//! `/v1/admin`: what the admin page needs beyond the regular API. Service
//! status, the settings file, restarts, the server certificate and backups.
//! Admin scope only, and only while `admin_ui` is on.

use std::net::IpAddr;
use std::path::PathBuf;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqail_proto::Scope;
use utoipa::ToSchema;

use crate::auth::Principal;
use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::AuditEvent;
use crate::{Restart, tls};

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AdminStatus {
    pub version: String,
    /// When the service process started (restarts from the admin page keep it).
    pub started_at: DateTime<Utc>,
    /// The address the service listens on.
    pub listen: String,
    /// Addresses clients can use (for a wildcard address: this host's name
    /// and loopback).
    pub urls: Vec<String>,
    /// Only this machine can connect.
    pub loopback_only: bool,
    /// SHA-256 of the served certificate, as clients pin it.
    pub fingerprint: String,
    pub self_signed: bool,
    pub mutual_tls: bool,
    pub sqlite_enabled: bool,
    pub data_dir: String,
    pub config_file: String,
    pub connections: usize,
    pub active_tokens: usize,
    pub sessions: usize,
    pub running_queries: usize,
    /// Why the last restart from the admin page failed, if it did.
    pub last_restart_error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AdminSettings {
    /// The settings as saved in the config file (without environment overrides).
    pub settings: Config,
    /// Keys an `SQAIL_*` environment variable replaces; changing them in the
    /// file has no effect.
    pub env_overrides: Vec<String>,
    pub config_file: String,
    pub data_dir: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Restarting {
    /// The port the service will listen on after the restart.
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CertificateUpload {
    /// PEM certificate chain, leaf first.
    pub cert_pem: String,
    /// PEM private key (PKCS#8, PKCS#1 or SEC1).
    pub key_pem: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CertificateSaved {
    /// Use these as `tls.cert` and `tls.key`.
    pub cert: String,
    pub key: String,
    pub fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BackupDone {
    /// The backup file, on the service host. `master.key` is not included.
    pub path: String,
}

fn require(state: &AppState, p: &Principal) -> ApiResult<()> {
    if !state.config.admin_ui {
        return Err(ApiError::not_found("admin API"));
    }
    p.require(Scope::Admin)
}

fn audit(state: &AppState, p: &Principal, action: &str, detail: Option<String>) {
    state.store.audit(AuditEvent {
        actor: &p.name,
        action,
        target: None,
        detail,
        duration_ms: None,
        success: true,
    });
}

/// Service status for the admin page.
#[utoipa::path(get, path = "/v1/admin/status", tag = "admin",
    responses((status = 200, body = AdminStatus)))]
pub async fn status(State(state): State<AppState>, p: Principal) -> ApiResult<Json<AdminStatus>> {
    require(&state, &p)?;
    let bind = state.config.bind;
    let port = bind.port();
    let urls = if bind.ip().is_unspecified() {
        let mut urls = vec![];
        if let Some(host) = host_name() {
            urls.push(format!("https://{host}:{port}"));
        }
        urls.push(crate::local_url(bind));
        urls
    } else {
        vec![crate::local_url(bind)]
    };
    let running_queries = state
        .queries
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len();
    Ok(Json(AdminStatus {
        version: env!("CARGO_PKG_VERSION").into(),
        started_at: state.control.started_at,
        listen: bind.to_string(),
        urls,
        loopback_only: bind.ip().is_loopback(),
        fingerprint: state.tls.fingerprint.clone(),
        self_signed: state.tls.self_signed,
        mutual_tls: state.config.tls.client_ca.is_some(),
        sqlite_enabled: !state.config.sqlite.allowed_dirs.is_empty(),
        data_dir: state.config.data_dir.display().to_string(),
        config_file: state.config.config_path().display().to_string(),
        connections: state.store.connection_list()?.len(),
        active_tokens: state
            .store
            .token_list()?
            .iter()
            .filter(|t| !t.revoked)
            .count(),
        sessions: state.sessions.len(),
        running_queries,
        last_restart_error: state.control.last_error(),
    }))
}

/// The settings in the config file.
#[utoipa::path(get, path = "/v1/admin/settings", tag = "admin",
    responses((status = 200, body = AdminSettings)))]
pub async fn settings(
    State(state): State<AppState>,
    p: Principal,
) -> ApiResult<Json<AdminSettings>> {
    require(&state, &p)?;
    Ok(Json(AdminSettings {
        settings: state.config.file_settings()?,
        env_overrides: Config::env_overrides()
            .into_iter()
            .map(String::from)
            .collect(),
        config_file: state.config.config_path().display().to_string(),
        data_dir: state.config.data_dir.display().to_string(),
    }))
}

/// Check new settings, then restart the service with them. They are saved
/// to the config file once the service runs with them; if it cannot start,
/// it goes back to the previous settings and `/v1/admin/status` reports why.
/// Open sessions are closed and running queries cancelled.
#[utoipa::path(put, path = "/v1/admin/settings", tag = "admin", request_body = Config,
    responses((status = 202, body = Restarting), (status = 400, body = sqail_proto::Problem)))]
pub async fn update_settings(
    State(state): State<AppState>,
    p: Principal,
    Json(file): Json<Config>,
) -> ApiResult<(StatusCode, Json<Restarting>)> {
    require(&state, &p)?;
    file.check().map_err(ApiError::bad_request)?;
    let effective = state.config.with_file_settings(file.clone())?;
    preflight(&state, &effective).map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    let port = effective.bind.port();
    audit(
        &state,
        &p,
        "settings.update",
        Some(format!("bind {}", effective.bind)),
    );
    state.control.request(Restart::Apply {
        effective: Box::new(effective),
        file: Box::new(file),
    });
    Ok((StatusCode::ACCEPTED, Json(Restarting { port })))
}

/// Everything that can be checked without stopping the running server.
fn preflight(state: &AppState, cfg: &Config) -> anyhow::Result<()> {
    use anyhow::Context;
    for dir in &cfg.sqlite.allowed_dirs {
        anyhow::ensure!(
            dir.is_absolute() && dir.is_dir(),
            "SQLite folder {} does not exist on the service host (use an absolute path)",
            dir.display()
        );
    }
    tls::load(cfg).context("TLS")?;
    // A new port can be tried now. The current port is still ours, so for
    // it only check that the address belongs to this machine.
    if cfg.bind.port() != state.config.bind.port() {
        std::net::TcpListener::bind(cfg.bind)
            .with_context(|| format!("cannot listen on {}", cfg.bind))?;
    } else if !is_local_ip(cfg.bind.ip()) {
        anyhow::bail!("{} is not an address of this machine", cfg.bind.ip());
    }
    // Fail now rather than after the restart when the file can't be written.
    let path = cfg.config_path();
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Whether the service could bind `ip` (probed on an ephemeral port).
fn is_local_ip(ip: IpAddr) -> bool {
    ip.is_unspecified() || std::net::TcpListener::bind((ip, 0)).is_ok()
}

/// Re-read the config file and restart.
#[utoipa::path(post, path = "/v1/admin/restart", tag = "admin",
    responses((status = 202, body = Restarting)))]
pub async fn restart(
    State(state): State<AppState>,
    p: Principal,
) -> ApiResult<(StatusCode, Json<Restarting>)> {
    require(&state, &p)?;
    let next = state
        .config
        .with_file_settings(state.config.file_settings()?)?;
    audit(&state, &p, "service.restart", None);
    state.control.request(Restart::Reload);
    Ok((
        StatusCode::ACCEPTED,
        Json(Restarting {
            port: next.bind.port(),
        }),
    ))
}

/// Store a certificate and key in `<data-dir>/tls/`. They take effect once
/// the settings point `tls.cert` and `tls.key` at the returned paths.
#[utoipa::path(post, path = "/v1/admin/certificate", tag = "admin", request_body = CertificateUpload,
    responses((status = 200, body = CertificateSaved), (status = 400, body = sqail_proto::Problem)))]
pub async fn upload_certificate(
    State(state): State<AppState>,
    p: Principal,
    Json(body): Json<CertificateUpload>,
) -> ApiResult<Json<CertificateSaved>> {
    require(&state, &p)?;
    let fingerprint = tls::check_pair(body.cert_pem.as_bytes(), body.key_pem.as_bytes())
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    let dir = state.config.data_dir.join("tls");
    let (cert, key): (PathBuf, PathBuf) = (dir.join("server-cert.pem"), dir.join("server-key.pem"));
    std::fs::create_dir_all(&dir).map_err(ApiError::internal)?;
    std::fs::write(&cert, body.cert_pem).map_err(ApiError::internal)?;
    // A fresh file, so it gets owner-only permissions.
    let _ = std::fs::remove_file(&key);
    crate::crypto::write_private_file(&key, body.key_pem.as_bytes())?;
    audit(&state, &p, "certificate.upload", Some(fingerprint.clone()));
    Ok(Json(CertificateSaved {
        cert: cert.display().to_string(),
        key: key.display().to_string(),
        fingerprint,
    }))
}

/// Write a consistent copy of `service.db` to `<data-dir>/backups/`.
#[utoipa::path(post, path = "/v1/admin/backup", tag = "admin",
    responses((status = 200, body = BackupDone)))]
pub async fn backup(State(state): State<AppState>, p: Principal) -> ApiResult<Json<BackupDone>> {
    require(&state, &p)?;
    let dir = state.config.data_dir.join("backups");
    std::fs::create_dir_all(&dir).map_err(ApiError::internal)?;
    let path = dir.join(format!("service-{}.db", Utc::now().format("%Y%m%d-%H%M%S")));
    state.store.backup_to(&path)?;
    audit(&state, &p, "backup", Some(path.display().to_string()));
    Ok(Json(BackupDone {
        path: path.display().to_string(),
    }))
}

fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|h| !h.is_empty())
        .map(|h| h.to_lowercase())
}

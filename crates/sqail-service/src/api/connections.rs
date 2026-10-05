use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use sqail_proto::{
    Connection, ConnectionInput, ConnectionParams, MssqlAuth, NamedItem, PgSslMode, Scope,
    TestResult,
};
use utoipa::IntoParams;
use uuid::Uuid;

use super::{decrypt_password, decrypt_ssl_key, load_connection};
use crate::auth::Principal;
use crate::engine::registry::{DriverSpec, build_driver};
use crate::engine::sqlite::check_path;
use crate::engine::tls_client::{self, PEM_LIMIT};
use crate::engine::{DbError, introspect};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::{AuditEvent, ConnectionRecord, StoredConnection};

#[utoipa::path(get, path = "/v1/connections", tag = "connections",
    responses((status = 200, body = Vec<Connection>)))]
pub async fn list(
    State(state): State<AppState>,
    _p: Principal,
) -> ApiResult<Json<Vec<Connection>>> {
    Ok(Json(state.store.connection_list()?))
}

#[utoipa::path(get, path = "/v1/connections/{id}", tag = "connections",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Connection), (status = 404, body = sqail_proto::Problem)))]
pub async fn get(
    State(state): State<AppState>,
    _p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(load_connection(&state, id)?.info))
}

fn validate(state: &AppState, input: &ConnectionInput) -> ApiResult<()> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 200 {
        return Err(ApiError::bad_request("name must be 1-200 characters"));
    }
    if let Some(c) = &input.color
        && !(c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()))
    {
        return Err(ApiError::bad_request("color must look like #rrggbb"));
    }
    match &input.params {
        ConnectionParams::Postgres(p) if p.host.is_empty() || p.database.is_empty() => {
            Err(ApiError::bad_request("host and database are required"))
        }
        ConnectionParams::Mssql(p) if p.host.is_empty() => {
            Err(ApiError::bad_request("host is required"))
        }
        ConnectionParams::Mssql(p) => match &p.auth {
            MssqlAuth::EntraPassword { user, .. } if user.trim().is_empty() => {
                Err(ApiError::bad_request("user is required"))
            }
            MssqlAuth::EntraServicePrincipal { tenant, client_id }
                if tenant.trim().is_empty() || client_id.trim().is_empty() =>
            {
                Err(ApiError::bad_request("tenant and client_id are required"))
            }
            _ => Ok(()),
        },
        ConnectionParams::Sqlite(p) => {
            check_path(
                std::path::Path::new(&p.path),
                &state.config.sqlite.allowed_dirs,
            )?;
            Ok(())
        }
        _ => Ok(()),
    }
}

fn record<'a>(
    input: &'a ConnectionInput,
    secret: Option<&'a str>,
    ssl_key: Option<&'a str>,
) -> ConnectionRecord<'a> {
    ConnectionRecord {
        name: input.name.trim(),
        params: &input.params,
        secret,
        ssl_key,
        read_only: input.read_only,
        color: input.color.as_deref(),
        environment: input.environment.as_deref(),
        folder: input.folder.as_deref(),
    }
}

#[utoipa::path(post, path = "/v1/connections", tag = "connections", request_body = ConnectionInput,
    responses((status = 201, body = Connection), (status = 400, body = sqail_proto::Problem)))]
pub async fn create(
    State(state): State<AppState>,
    p: Principal,
    Json(input): Json<ConnectionInput>,
) -> ApiResult<(StatusCode, Json<Connection>)> {
    p.require(Scope::Admin)?;
    let mut input = input;
    normalize_pg(&mut input)?;
    validate(&state, &input)?;
    let ssl = resolve_ssl_key(&state, &input, None)?;
    let secret = match input.password.as_deref() {
        Some(pw) if !pw.is_empty() => Some(state.key.encrypt(pw)?),
        _ => None,
    };
    let id =
        state
            .store
            .connection_insert(&record(&input, secret.as_deref(), ssl.stored.as_deref()))?;
    audit(&state, &p, "connection.create", id, &input.name);
    Ok((StatusCode::CREATED, Json(load_connection(&state, id)?.info)))
}

#[utoipa::path(put, path = "/v1/connections/{id}", tag = "connections", request_body = ConnectionInput,
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Connection), (status = 404, body = sqail_proto::Problem)))]
pub async fn update(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Json(input): Json<ConnectionInput>,
) -> ApiResult<Json<Connection>> {
    p.require(Scope::Admin)?;
    let mut input = input;
    normalize_pg(&mut input)?;
    validate(&state, &input)?;
    let existing = load_connection(&state, id)?;
    let ssl = resolve_ssl_key(&state, &input, Some(&existing))?;
    let secret = match input.password.as_deref() {
        None => existing.secret,
        Some("") => None,
        Some(pw) => Some(state.key.encrypt(pw)?),
    };
    state.store.connection_update(
        id,
        &record(&input, secret.as_deref(), ssl.stored.as_deref()),
    )?;
    state.pools.invalidate(id);
    audit(&state, &p, "connection.update", id, &input.name);
    Ok(Json(load_connection(&state, id)?.info))
}

#[utoipa::path(delete, path = "/v1/connections/{id}", tag = "connections",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = sqail_proto::Problem)))]
pub async fn delete(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    p.require(Scope::Admin)?;
    let existing = load_connection(&state, id)?;
    state.store.connection_delete(id)?;
    state.pools.invalidate(id);
    state.sessions.close_for_connection(id);
    audit(&state, &p, "connection.delete", id, &existing.info.name);
    Ok(StatusCode::NO_CONTENT)
}

/// Open a fresh connection with a stored profile and report the server version.
#[utoipa::path(post, path = "/v1/connections/{id}/test", tag = "connections",
    params(("id" = Uuid, Path)), responses((status = 200, body = TestResult)))]
pub async fn test_saved(
    State(state): State<AppState>,
    _p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TestResult>> {
    let stored = load_connection(&state, id)?;
    let password = decrypt_password(&state, &stored)?;
    let ssl_key = decrypt_ssl_key(&state, &stored)?;
    Ok(Json(
        run_test(
            &state,
            &stored.info.params,
            password.as_deref(),
            ssl_key.as_deref(),
            stored.info.read_only,
        )
        .await,
    ))
}

/// Test a profile before saving it. Admin only: it connects to arbitrary hosts.
#[utoipa::path(post, path = "/v1/connections/test", tag = "connections", request_body = ConnectionInput,
    params(SecretFromQuery),
    responses((status = 200, body = TestResult), (status = 400, body = sqail_proto::Problem)))]
pub async fn test_unsaved(
    State(state): State<AppState>,
    p: Principal,
    Query(q): Query<SecretFromQuery>,
    Json(mut input): Json<ConnectionInput>,
) -> ApiResult<Json<TestResult>> {
    p.require(Scope::Admin)?;
    let (password, ssl_key) = prepare_unsaved(&state, &mut input, q.secret_from)?;
    Ok(Json(
        run_test(
            &state,
            &input.params,
            password.as_deref(),
            ssl_key.plain.as_deref(),
            input.read_only,
        )
        .await,
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SecretFromQuery {
    /// When the body omits `password` or `ssl_client_key`, use the secret
    /// stored with this profile (the connection form while editing).
    secret_from: Option<Uuid>,
}

/// The databases the login in a (possibly unsaved) profile can access, for
/// picking one in the connection form. Admin only: it connects to arbitrary
/// hosts. PostgreSQL connects to the profile's database, or `postgres` when
/// it is empty.
#[utoipa::path(post, path = "/v1/connections/databases", tag = "connections",
    request_body = ConnectionInput, params(SecretFromQuery),
    responses((status = 200, body = Vec<NamedItem>), (status = 502, body = sqail_proto::Problem)))]
pub async fn databases_unsaved(
    State(state): State<AppState>,
    p: Principal,
    Query(q): Query<SecretFromQuery>,
    Json(mut input): Json<ConnectionInput>,
) -> ApiResult<Json<Vec<NamedItem>>> {
    p.require(Scope::Admin)?;
    // The form may not have a name yet; nothing is stored.
    if input.name.trim().is_empty() {
        input.name = "unsaved".into();
    }
    if let ConnectionParams::Postgres(pg) = &mut input.params
        && pg.database.trim().is_empty()
    {
        pg.database = "postgres".into();
    }
    let (password, ssl_key) = prepare_unsaved(&state, &mut input, q.secret_from)?;
    let res = async {
        let driver = build_driver(&DriverSpec {
            params: &input.params,
            password: password.as_deref(),
            ssl_client_key: ssl_key.plain.as_deref(),
            read_only: input.read_only,
            sqlite_dirs: &state.config.sqlite.allowed_dirs,
        })?;
        let mut conn = driver.connect().await?;
        introspect::databases(conn.as_mut()).await
    };
    match tokio::time::timeout(Duration::from_secs(20), res).await {
        Ok(r) => Ok(Json(r?)),
        Err(_) => Err(DbError::Timeout.into()),
    }
}

struct ResolvedKey {
    plain: Option<String>,
    /// Already encrypted, ready for `service.db`.
    stored: Option<String>,
}

/// Trim certificate PEMs and the client key. An empty key stays empty: on
/// update that clears the stored key.
fn normalize_pg(input: &mut ConnectionInput) -> ApiResult<()> {
    if let Some(key) = &mut input.ssl_client_key {
        let trimmed = key.trim();
        if trimmed.len() > PEM_LIMIT {
            return Err(ApiError::bad_request("client key is larger than 64 KiB"));
        }
        if trimmed.len() != key.len() {
            *key = trimmed.to_string();
        }
    }
    let ConnectionParams::Postgres(p) = &mut input.params else {
        return Ok(());
    };
    p.ssl_root_cert = tidy_pem(p.ssl_root_cert.take(), "CA certificate")?;
    p.ssl_client_cert = tidy_pem(p.ssl_client_cert.take(), "client certificate")?;
    Ok(())
}

fn tidy_pem(value: Option<String>, what: &str) -> ApiResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > PEM_LIMIT {
        return Err(ApiError::bad_request(format!(
            "{what} is larger than 64 KiB"
        )));
    }
    Ok(Some(value.to_string()))
}

/// The client key to use, and the ciphertext to store. Builds the TLS config
/// so a bad PEM fails here, before anything is written.
fn resolve_ssl_key(
    state: &AppState,
    input: &ConnectionInput,
    existing: Option<&StoredConnection>,
) -> ApiResult<ResolvedKey> {
    let ConnectionParams::Postgres(p) = &input.params else {
        if input.ssl_client_key.as_ref().is_some_and(|k| !k.is_empty()) {
            return Err(ApiError::bad_request(
                "ssl_client_key is only for PostgreSQL",
            ));
        }
        return Ok(ResolvedKey {
            plain: None,
            stored: None,
        });
    };

    let cert = p.ssl_client_cert.as_deref();
    let key = input.ssl_client_key.as_deref();
    // Guards on `&str` are not exhaustive, so this is written as ifs.
    let (plain, stored) = if key.is_some_and(|k| !k.is_empty()) && cert.is_none() {
        return Err(ApiError::bad_request(
            "a client key needs a client certificate",
        ));
    } else if let Some(key) = key.filter(|k| !k.is_empty()) {
        (Some(key.to_string()), None)
    } else if cert.is_none() {
        // No certificate: drop a stored key too, including an explicit "".
        (None, None)
    } else if key.is_some() {
        return Err(ApiError::bad_request(
            "a client certificate needs its private key",
        ));
    } else {
        match existing.and_then(|e| e.ssl_key.clone()) {
            Some(enc) => {
                let plain = state.key.decrypt(&enc)?;
                (Some(plain), Some(enc))
            }
            None => {
                return Err(ApiError::bad_request(
                    "a client certificate needs its private key",
                ));
            }
        }
    };

    if p.ssl_mode == PgSslMode::Disable
        && (p.ssl_root_cert.is_some() || cert.is_some() || plain.is_some())
    {
        return Err(ApiError::bad_request(
            "SSL certificates need an SSL mode other than disable",
        ));
    }
    if p.ssl_root_cert.is_some()
        && !matches!(p.ssl_mode, PgSslMode::VerifyCa | PgSslMode::VerifyFull)
    {
        return Err(ApiError::bad_request(
            "a CA certificate is only used with ssl_mode verify-ca or verify-full",
        ));
    }

    tls_client::for_postgres(tls_client::PgTls {
        mode: p.ssl_mode,
        root_pem: p.ssl_root_cert.as_deref(),
        client_cert_pem: cert,
        client_key_pem: plain.as_deref(),
    })
    .map_err(ApiError::bad_request)?;

    let stored = match input.ssl_client_key.as_deref() {
        Some(key) if !key.is_empty() => Some(state.key.encrypt(key)?),
        _ => stored,
    };
    Ok(ResolvedKey { plain, stored })
}

fn prepare_unsaved(
    state: &AppState,
    input: &mut ConnectionInput,
    secret_from: Option<Uuid>,
) -> ApiResult<(Option<String>, ResolvedKey)> {
    normalize_pg(input)?;
    validate(state, input)?;
    let stored = secret_from
        .map(|id| load_connection(state, id))
        .transpose()?;
    let password = match input.password.as_deref() {
        Some(pw) => Some(pw.to_string()),
        None => match &stored {
            Some(saved) => decrypt_password(state, saved)?,
            None => None,
        },
    };
    let ssl = resolve_ssl_key(state, input, stored.as_ref())?;
    Ok((password, ssl))
}

async fn run_test(
    state: &AppState,
    params: &ConnectionParams,
    password: Option<&str>,
    ssl_client_key: Option<&str>,
    read_only: bool,
) -> TestResult {
    let started = Instant::now();
    let res = async {
        let driver = build_driver(&DriverSpec {
            params,
            password,
            ssl_client_key,
            read_only,
            sqlite_dirs: &state.config.sqlite.allowed_dirs,
        })?;
        let mut conn = driver.connect().await?;
        conn.server_version().await
    };
    let res = tokio::time::timeout(Duration::from_secs(20), res).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match res {
        Ok(Ok(version)) => TestResult {
            ok: true,
            latency_ms,
            server_version: Some(version),
            error: None,
        },
        Ok(Err(e)) => TestResult {
            ok: false,
            latency_ms,
            server_version: None,
            error: Some(e.to_string()),
        },
        Err(_) => TestResult {
            ok: false,
            latency_ms,
            server_version: None,
            error: Some("timed out after 20s".into()),
        },
    }
}

fn audit(state: &AppState, p: &Principal, action: &str, id: Uuid, name: &str) {
    state.store.audit(AuditEvent {
        actor: &p.name,
        action,
        target: Some(id.to_string()),
        detail: Some(name.to_string()),
        duration_ms: None,
        success: true,
    });
}

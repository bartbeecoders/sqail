//! sqail-service: an HTTPS REST gateway between sqail and SQL databases.
//!
//! [`run`] serves until shutdown, restarting in place when the admin page
//! changes the settings; [`start`] boots a single server. `main.rs` is a thin
//! CLI around them, and the integration tests use both in-process.

pub mod api;
pub mod auth;
pub mod azure;
pub mod config;
pub mod crypto;
pub mod engine;
pub mod error;
pub mod sessions;
pub mod state;
pub mod store;
pub mod tls;

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use chrono::{DateTime, Utc};
use sqail_proto::Scope;
use tokio::sync::Notify;

pub use config::Config;

use crate::crypto::MasterKey;
use crate::state::AppState;
use crate::store::{AuditEvent, Store};

/// A running server.
pub struct Server {
    pub addr: SocketAddr,
    /// SHA-256 fingerprint of the served certificate (for client pinning).
    pub fingerprint: String,
    /// Set on a start with no active admin token: the new admin token,
    /// shown exactly once.
    pub bootstrap_token: Option<String>,
    handle: Handle<SocketAddr>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Server {
    /// Stop accepting connections and let in-flight requests finish (≤10 s).
    pub fn shutdown(&self) {
        self.handle.graceful_shutdown(Some(Duration::from_secs(10)));
    }

    pub async fn wait(self) -> Result<()> {
        self.task.await.context("server task panicked")??;
        Ok(())
    }
}

/// What the admin API asks [`run`] to do.
pub enum Restart {
    /// Re-read the config file and restart.
    Reload,
    /// Restart with `effective`; once it is up, save `file` as the config file.
    Apply {
        effective: Box<Config>,
        file: Box<Config>,
    },
}

/// Shared between [`run`] and the servers it starts: restart requests from
/// the admin API, and what the admin page shows about them.
pub struct Control {
    wake: Notify,
    pending: Mutex<Option<Restart>>,
    last_error: Mutex<Option<String>>,
    pub started_at: DateTime<Utc>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            wake: Notify::new(),
            pending: Mutex::new(None),
            last_error: Mutex::new(None),
            started_at: Utc::now(),
        }
    }
}

impl Control {
    /// Ask [`run`] to restart. A later request replaces an unhandled one.
    pub fn request(&self, restart: Restart) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(restart);
        self.wake.notify_one();
    }

    /// Why the last restart failed, until one succeeds.
    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_error(&self, error: Option<String>) {
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = error;
    }

    fn take(&self) -> Option<Restart> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

/// Boot one server with its own [`Control`] (restart requests are ignored).
pub async fn start(config: Config) -> Result<Server> {
    start_with(config, Arc::new(Control::default())).await
}

/// Serve until `shutdown` resolves. Restart requests from the admin API
/// stop the server and start it again with the new settings; the settings
/// are saved only once the server is up with them. When that start fails,
/// the previous settings are restored and the error is kept for the admin
/// page. `on_start` sees every server started, including after restarts.
pub async fn run(
    config: Config,
    reload: impl Fn() -> Result<Config>,
    shutdown: impl Future<Output = ()>,
    mut on_start: impl FnMut(&Server) -> Result<()>,
) -> Result<()> {
    let control = Arc::new(Control::default());
    let mut current = config;
    let mut server = start_with(current.clone(), control.clone()).await?;
    on_start(&server)?;
    // Only the very first start may create the bootstrap admin token.
    current.bootstrap_admin_token = false;
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = control.wake.notified() => {}
        }
        let Some(request) = control.take() else {
            continue;
        };
        let (mut next, save) = match request {
            Restart::Reload => match reload() {
                Ok(c) => (c, None),
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "not restarting: cannot read the settings");
                    control.set_error(Some(format!("{e:#}")));
                    continue;
                }
            },
            Restart::Apply { effective, file } => (*effective, Some(file)),
        };
        next.bootstrap_admin_token = false;
        tracing::info!("restarting with new settings");
        server.shutdown();
        server.wait().await?;
        server = match start_with(next.clone(), control.clone()).await {
            Ok(s) => {
                let saved = match save {
                    Some(file) => save_settings(&next, &file),
                    None => Ok(()),
                };
                control.set_error(saved.err().map(|e| format!("{e:#}")));
                current = next;
                s
            }
            Err(e) => {
                let msg = format!("could not start with the new settings: {e:#}");
                tracing::error!(error = %msg, "keeping the previous settings");
                control.set_error(Some(msg));
                start_with(current.clone(), control.clone())
                    .await
                    .context("restarting with the previous settings")?
            }
        };
        on_start(&server)?;
    }
    tracing::info!("shutting down");
    server.shutdown();
    server.wait().await
}

fn save_settings(effective: &Config, file: &Config) -> Result<()> {
    let path = effective.config_path();
    std::fs::write(&path, file.to_toml()?).with_context(|| {
        format!(
            "the new settings are active but could not be saved to {}",
            path.display()
        )
    })?;
    tracing::info!(path = %path.display(), "saved settings");
    Ok(())
}

async fn start_with(config: Config, control: Arc<Control>) -> Result<Server> {
    let tls = tls::load(&config)?;
    let store = Store::open(&config.db_path())?;
    let key = MasterKey::load_or_create(&config.data_dir)?;

    let bootstrap_token = if config.bootstrap_admin_token && store.active_admin_tokens()? == 0 {
        let created = store.token_create("bootstrap-admin", Scope::Admin)?;
        store.audit(AuditEvent {
            actor: "service",
            action: "token.create",
            target: Some(created.info.id.to_string()),
            detail: Some("bootstrap admin token".into()),
            duration_ms: None,
            success: true,
        });
        Some(created.token)
    } else {
        None
    };

    let fingerprint = tls.fingerprint.clone();
    let state = AppState::new(
        config,
        store,
        key,
        state::TlsInfo {
            fingerprint: tls.fingerprint,
            self_signed: tls.self_signed,
        },
        control,
    );
    let reaper = spawn_session_reaper(state.clone());
    let app = api::router(state.clone());

    let handle = Handle::new();
    // TCP_NODELAY: without it, Nagle + delayed ACK add ~40 ms to small responses.
    let acceptor =
        axum_server::tls_rustls::RustlsAcceptor::new(RustlsConfig::from_config(tls.config))
            .acceptor(axum_server::accept::NoDelayAcceptor::new());
    let server = axum_server::bind(state.config.bind)
        .acceptor(acceptor)
        .handle(handle.clone());
    let task = tokio::spawn(async move {
        let res = server.serve(app.into_make_service()).await;
        reaper.abort();
        res
    });
    let addr = match handle.listening().await {
        Some(addr) => addr,
        None => {
            // Surface the bind error rather than a generic message.
            let detail = match task.await {
                Ok(Err(e)) => e.to_string(),
                _ => "unknown error".into(),
            };
            anyhow::bail!("could not listen on {}: {detail}", state.config.bind);
        }
    };

    Ok(Server {
        addr,
        fingerprint,
        bootstrap_token,
        handle,
        task,
    })
}

/// The address to reach a server listening on `bind` from this machine.
pub fn local_url(bind: SocketAddr) -> String {
    let host = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => "[::1]".to_string(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    format!("https://{host}:{}", bind.port())
}

/// A link that opens the admin page signed in with `token`. The token is in
/// the fragment, so browsers never send it to the server or log it.
pub fn admin_link(bind: SocketAddr, token: &str) -> String {
    format!("{}/admin/#token={token}", local_url(bind))
}

fn spawn_session_reaper(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let closed = state.sessions.reap(state.config.sessions.idle_timeout_secs);
            if closed > 0 {
                tracing::info!(closed, "closed idle sessions");
            }
        }
    })
}

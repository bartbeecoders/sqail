//! sqail-service: an HTTPS REST gateway between sqail2 and SQL databases.
//!
//! [`start`] boots the server; `main.rs` is a thin CLI around it, and the
//! integration tests start it in-process.

pub mod api;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod engine;
pub mod error;
pub mod sessions;
pub mod state;
pub mod store;
pub mod tls;

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use sqail_proto::Scope;

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

pub async fn start(config: Config) -> Result<Server> {
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

    let state = AppState::new(config, store, key);
    spawn_session_reaper(state.clone());
    let app = api::router(state.clone());

    let handle = Handle::new();
    // TCP_NODELAY: without it, Nagle + delayed ACK add ~40 ms to small responses.
    let acceptor =
        axum_server::tls_rustls::RustlsAcceptor::new(RustlsConfig::from_config(tls.config))
            .acceptor(axum_server::accept::NoDelayAcceptor::new());
    let server = axum_server::bind(state.config.bind)
        .acceptor(acceptor)
        .handle(handle.clone());
    let task = tokio::spawn(server.serve(app.into_make_service()));
    let addr = handle
        .listening()
        .await
        .with_context(|| format!("could not listen on {}", state.config.bind))?;

    Ok(Server {
        addr,
        fingerprint: tls.fingerprint,
        bootstrap_token,
        handle,
        task,
    })
}

fn spawn_session_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let closed = state.sessions.reap(state.config.sessions.idle_timeout_secs);
            if closed > 0 {
                tracing::info!(closed, "closed idle sessions");
            }
        }
    });
}

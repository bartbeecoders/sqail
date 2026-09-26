//! Shared application state.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use governor::DefaultKeyedRateLimiter;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::Config;
use crate::crypto::MasterKey;
use crate::engine::registry::Registry;
use crate::sessions::Sessions;
use crate::store::Store;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub config: Config,
    pub store: Store,
    pub key: MasterKey,
    pub limiter: DefaultKeyedRateLimiter<Uuid>,
    pub pools: Registry,
    pub sessions: Sessions,
    pub queries: Mutex<HashMap<Uuid, RunningQuery>>,
    /// Throttles `last_used_at` writes to one per token per minute.
    token_touched: Mutex<HashMap<Uuid, Instant>>,
}

pub struct RunningQuery {
    pub owner: Uuid,
    pub cancel: CancellationToken,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(config: Config, store: Store, key: MasterKey) -> Self {
        let limiter =
            crate::auth::rate_limiter(config.limits.requests_per_second, config.limits.burst);
        Self(Arc::new(Inner {
            config,
            store,
            key,
            limiter,
            pools: Registry::default(),
            sessions: Sessions::default(),
            queries: Mutex::new(HashMap::new()),
            token_touched: Mutex::new(HashMap::new()),
        }))
    }

    pub fn touch_token(&self, id: Uuid) {
        let mut touched = self.token_touched.lock().unwrap_or_else(|e| e.into_inner());
        let due = touched
            .get(&id)
            .is_none_or(|t| t.elapsed() > Duration::from_secs(60));
        if due {
            touched.insert(id, Instant::now());
            if let Err(e) = self.store.token_touch(id) {
                tracing::warn!(error = %e, "could not update token last_used_at");
            }
        }
    }
}

use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::ai::provider::{AiHistoryEntry, AiProviderConfig};
use crate::ai::store::{AiHistoryStore, AiProviderStore};
use crate::db::connections::ConnectionConfig;
use crate::db::store::ConnectionStore;
use crate::pool::DbPool;

pub struct AppState {
    pub store: ConnectionStore,
    pub connections: Mutex<Vec<ConnectionConfig>>,
    pub pools: Mutex<HashMap<String, DbPool>>,
    pub active_connection_id: Mutex<Option<String>>,
    /// Entra ID access tokens keyed by connection ID
    pub entra_tokens: Mutex<HashMap<String, String>>,

    pub ai_provider_store: AiProviderStore,
    pub ai_providers: Mutex<Vec<AiProviderConfig>>,
    pub ai_history_store: AiHistoryStore,
    pub ai_history: Mutex<Vec<AiHistoryEntry>>,
}

impl AppState {
    pub fn new(
        store: ConnectionStore,
        ai_provider_store: AiProviderStore,
        ai_history_store: AiHistoryStore,
    ) -> Self {
        let connections = store.load();
        let ai_providers = ai_provider_store.load();
        let ai_history = ai_history_store.load();
        Self {
            store,
            connections: Mutex::new(connections),
            pools: Mutex::new(HashMap::new()),
            active_connection_id: Mutex::new(None),
            entra_tokens: Mutex::new(HashMap::new()),
            ai_provider_store,
            ai_providers: Mutex::new(ai_providers),
            ai_history_store,
            ai_history: Mutex::new(ai_history),
        }
    }

    pub async fn save(&self) -> Result<(), String> {
        let conns = self.connections.lock().await;
        self.store.save(&conns)
    }

    pub async fn save_ai_providers(&self) -> Result<(), String> {
        let providers = self.ai_providers.lock().await;
        self.ai_provider_store.save(&providers)
    }

    pub async fn save_ai_history(&self) -> Result<(), String> {
        let history = self.ai_history.lock().await;
        self.ai_history_store.save(&history)
    }
}

use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::ai::inline::state::InlineAiState;
use crate::ai::provider::{AiHistoryEntry, AiProviderConfig};
use crate::ai::store::{AiHistoryStore, AiProviderStore};
use crate::db::connections::ConnectionConfig;
use crate::db::store::ConnectionStore;
use crate::metadata::{MetadataStore, ObjectMetadata};
use crate::pool::DbPool;
use crate::query_history::{
    QueryHistoryEntry, QueryHistoryStore, SavedQuery, SavedQueryStore,
};

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

    pub query_history_store: QueryHistoryStore,
    pub query_history: Mutex<Vec<QueryHistoryEntry>>,
    pub saved_query_store: SavedQueryStore,
    pub saved_queries: Mutex<Vec<SavedQuery>>,

    pub metadata_store: MetadataStore,
    pub metadata: Mutex<Vec<ObjectMetadata>>,

    pub inline_ai: InlineAiState,
}

impl AppState {
    pub fn new(
        store: ConnectionStore,
        ai_provider_store: AiProviderStore,
        ai_history_store: AiHistoryStore,
        query_history_store: QueryHistoryStore,
        saved_query_store: SavedQueryStore,
        metadata_store: MetadataStore,
    ) -> Self {
        let connections = store.load();
        let ai_providers = ai_provider_store.load();
        let ai_history = ai_history_store.load();
        let query_history = query_history_store.load();
        let saved_queries = saved_query_store.load();
        let metadata = metadata_store.load();
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
            query_history_store,
            query_history: Mutex::new(query_history),
            saved_query_store,
            saved_queries: Mutex::new(saved_queries),
            metadata_store,
            metadata: Mutex::new(metadata),
            inline_ai: InlineAiState::new(),
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

    pub async fn save_query_history(&self) -> Result<(), String> {
        let history = self.query_history.lock().await;
        self.query_history_store.save(&history)
    }

    pub async fn save_saved_queries(&self) -> Result<(), String> {
        let queries = self.saved_queries.lock().await;
        self.saved_query_store.save(&queries)
    }

    pub async fn save_metadata(&self) -> Result<(), String> {
        let metadata = self.metadata.lock().await;
        self.metadata_store.save(&metadata)
    }
}

use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::db::connections::ConnectionConfig;
use crate::db::store::ConnectionStore;
use crate::pool::DbPool;

pub struct AppState {
    pub store: ConnectionStore,
    pub connections: Mutex<Vec<ConnectionConfig>>,
    pub pools: Mutex<HashMap<String, DbPool>>,
    pub active_connection_id: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(store: ConnectionStore) -> Self {
        let connections = store.load();
        Self {
            store,
            connections: Mutex::new(connections),
            pools: Mutex::new(HashMap::new()),
            active_connection_id: Mutex::new(None),
        }
    }

    pub async fn save(&self) -> Result<(), String> {
        let conns = self.connections.lock().await;
        self.store.save(&conns)
    }
}

use std::path::PathBuf;

use crate::db::connections::ConnectionConfig;

/// Flat-file JSON store for connection configs in the app data directory.
pub struct ConnectionStore {
    path: PathBuf,
}

impl ConnectionStore {
    pub fn new(app_data_dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&app_data_dir)?;
        Ok(Self {
            path: app_data_dir.join("connections.json"),
        })
    }

    pub fn load(&self) -> Vec<ConnectionConfig> {
        match std::fs::read_to_string(&self.path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub fn save(&self, connections: &[ConnectionConfig]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(connections)
            .map_err(|e| format!("Failed to serialize: {e}"))?;
        std::fs::write(&self.path, json).map_err(|e| format!("Failed to write: {e}"))
    }
}

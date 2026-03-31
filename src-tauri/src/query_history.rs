use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_HISTORY_ENTRIES: usize = 500;

// ── Query History ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryHistoryEntry {
    pub id: String,
    pub timestamp: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub query: String,
    pub execution_time_ms: u64,
    pub row_count: Option<usize>,
    pub success: bool,
    pub error_message: Option<String>,
}

pub struct QueryHistoryStore {
    path: PathBuf,
}

impl QueryHistoryStore {
    pub fn new(app_data_dir: &PathBuf) -> Self {
        Self {
            path: app_data_dir.join("query_history.json"),
        }
    }

    pub fn load(&self) -> Vec<QueryHistoryEntry> {
        match std::fs::read_to_string(&self.path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub fn save(&self, history: &[QueryHistoryEntry]) -> Result<(), String> {
        let to_save = if history.len() > MAX_HISTORY_ENTRIES {
            &history[history.len() - MAX_HISTORY_ENTRIES..]
        } else {
            history
        };
        let json = serde_json::to_string_pretty(to_save)
            .map_err(|e| format!("Failed to serialize query history: {e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("Failed to write query history: {e}"))
    }
}

// ── Saved Queries ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub query: String,
    pub connection_id: Option<String>,
    pub tags: Vec<String>,
    pub folder: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct SavedQueryStore {
    path: PathBuf,
}

impl SavedQueryStore {
    pub fn new(app_data_dir: &PathBuf) -> Self {
        Self {
            path: app_data_dir.join("saved_queries.json"),
        }
    }

    pub fn load(&self) -> Vec<SavedQuery> {
        match std::fs::read_to_string(&self.path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub fn save(&self, queries: &[SavedQuery]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(queries)
            .map_err(|e| format!("Failed to serialize saved queries: {e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("Failed to write saved queries: {e}"))
    }
}

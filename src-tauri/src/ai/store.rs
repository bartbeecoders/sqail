use std::path::{Path, PathBuf};

use crate::ai::provider::{AiHistoryEntry, AiProviderConfig};

const MAX_HISTORY_ENTRIES: usize = 100;

/// JSON file store for AI provider configs.
pub struct AiProviderStore {
    path: PathBuf,
}

impl AiProviderStore {
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            path: app_data_dir.join("ai_providers.json"),
        }
    }

    pub fn load(&self) -> Vec<AiProviderConfig> {
        match std::fs::read_to_string(&self.path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub fn save(&self, providers: &[AiProviderConfig]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(providers)
            .map_err(|e| format!("Failed to serialize AI providers: {e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("Failed to write AI providers: {e}"))
    }
}

/// JSON file store for AI prompt history.
pub struct AiHistoryStore {
    path: PathBuf,
}

impl AiHistoryStore {
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            path: app_data_dir.join("ai_history.json"),
        }
    }

    pub fn load(&self) -> Vec<AiHistoryEntry> {
        match std::fs::read_to_string(&self.path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub fn save(&self, history: &[AiHistoryEntry]) -> Result<(), String> {
        // Cap at MAX_HISTORY_ENTRIES (keep most recent)
        let to_save = if history.len() > MAX_HISTORY_ENTRIES {
            &history[history.len() - MAX_HISTORY_ENTRIES..]
        } else {
            history
        };
        let json = serde_json::to_string_pretty(to_save)
            .map_err(|e| format!("Failed to serialize AI history: {e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("Failed to write AI history: {e}"))
    }
}

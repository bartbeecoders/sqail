use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMetadata {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedMetadata {
    pub description: String,
    pub columns: Vec<ColumnMetadata>,
    pub example_usage: String,
    pub related_objects: Vec<String>,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectMetadata {
    pub id: String,
    pub connection_id: String,
    pub schema_name: String,
    pub object_name: String,
    pub object_type: String, // "table", "view", "function", "procedure"
    pub metadata: GeneratedMetadata,
    pub generated_at: String,
    pub updated_at: String,
}

/// JSON file store for database object metadata.
pub struct MetadataStore {
    path: PathBuf,
}

impl MetadataStore {
    pub fn new(app_data_dir: &PathBuf) -> Self {
        Self {
            path: app_data_dir.join("metadata.json"),
        }
    }

    pub fn load(&self) -> Vec<ObjectMetadata> {
        match std::fs::read_to_string(&self.path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub fn save(&self, entries: &[ObjectMetadata]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(entries)
            .map_err(|e| format!("Failed to serialize metadata: {e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("Failed to write metadata: {e}"))
    }
}

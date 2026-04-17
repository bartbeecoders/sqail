//! Catalog of trained LoRA adapters.
//!
//! Each training run produces a directory under
//! `<app_data>/inline-ai/trained/<id>/` containing:
//!
//! * `adapter_model.safetensors` + `adapter_config.json` — the LoRA
//!   adapter in peft's default layout.
//! * `meta.json` — the sqail-side record (below).
//!
//! The on-disk layout is stable: third-party tooling (e.g. llama.cpp's
//! `convert_lora_to_gguf.py`) can read the adapter directory directly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainedModel {
    pub id: String,
    pub display_name: String,
    pub base_model_id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub dataset_size: u64,
    pub example_count: u32,
    pub table_count: u32,
    pub created_at: String,
    pub adapter_path: String,
    /// Optional — populated once the adapter has been converted to the
    /// llama.cpp `.gguf` format (via follow-up tooling).
    #[serde(default)]
    pub gguf_path: Option<String>,
}

pub fn trained_dir(app_data: &Path) -> PathBuf {
    app_data.join("inline-ai").join("trained")
}

pub fn model_dir(app_data: &Path, id: &str) -> PathBuf {
    trained_dir(app_data).join(id)
}

pub fn meta_path(app_data: &Path, id: &str) -> PathBuf {
    model_dir(app_data, id).join("meta.json")
}

pub async fn save(app_data: &Path, model: &TrainedModel) -> Result<(), String> {
    let dir = model_dir(app_data, &model.id);
    fs::create_dir_all(&dir).await.map_err(|e| format!("mkdir: {e}"))?;
    let json = serde_json::to_string_pretty(model).map_err(|e| e.to_string())?;
    fs::write(meta_path(app_data, &model.id), json)
        .await
        .map_err(|e| format!("write meta: {e}"))
}

pub async fn list(app_data: &Path) -> Vec<TrainedModel> {
    let dir = trained_dir(app_data);
    let mut out = Vec::new();
    let Ok(mut rd) = fs::read_dir(&dir).await else {
        return out;
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let meta = entry.path().join("meta.json");
        if let Ok(s) = fs::read_to_string(&meta).await {
            if let Ok(m) = serde_json::from_str::<TrainedModel>(&s) {
                out.push(m);
            }
        }
    }
    // Most recent first.
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

pub async fn delete(app_data: &Path, id: &str) -> Result<(), String> {
    let dir = model_dir(app_data, id);
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&dir)
        .await
        .map_err(|e| format!("rmdir {}: {e}", dir.display()))
}

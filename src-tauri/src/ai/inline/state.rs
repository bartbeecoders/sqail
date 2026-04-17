//! Top-level state for the inline AI feature — owned by `AppState`.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

use super::completer::CompletionRegistry;
use super::sidecar::SidecarManager;

/// Everything we need to keep alive for the lifetime of the app to serve
/// inline-AI related commands.
pub struct InlineAiState {
    pub sidecar: Arc<SidecarManager>,
    /// Cancel flags for in-flight model downloads, keyed by model id.
    pub downloads: Mutex<HashMap<String, Arc<Notify>>>,
    /// Cancel flag for the in-flight sidecar-binary download (if any).
    pub binary_download: Mutex<Option<Arc<Notify>>>,
    /// Registry of in-flight FIM completion requests.
    pub completions: Arc<CompletionRegistry>,
}

impl InlineAiState {
    pub fn new() -> Self {
        Self {
            sidecar: Arc::new(SidecarManager::new()),
            downloads: Mutex::new(HashMap::new()),
            binary_download: Mutex::new(None),
            completions: Arc::new(CompletionRegistry::new()),
        }
    }
}

impl Default for InlineAiState {
    fn default() -> Self {
        Self::new()
    }
}

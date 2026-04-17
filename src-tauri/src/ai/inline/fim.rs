//! Per-model FIM knobs for the `/infill` endpoint.
//!
//! llama.cpp's `/infill` handles the model-specific FIM *tokens*
//! automatically (it reads `tokenizer.ggml.fim_*` keys from the GGUF
//! metadata), so we only need to configure sampling and stop strings.
//!
//! Defaults come from the Phase A benchmarks — they produced on-schema
//! completions for every prompt on every candidate model with no extra
//! tuning. We still parameterise per-model so a future "reasoning" or
//! StarCoder variant that needs different knobs is easy to slot in.

use super::models::ModelEntry;

#[derive(Debug, Clone)]
pub struct FimConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub repeat_penalty: f32,
    pub n_predict: i32,
    /// Stop strings sent to the server. We additionally re-check these
    /// client-side so a buggy server stop doesn't produce runaways.
    pub stops: &'static [&'static str],
}

impl Default for FimConfig {
    fn default() -> Self {
        Self {
            temperature: 0.2,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.05,
            n_predict: 48,
            stops: &[";", "\n\n"],
        }
    }
}

/// Config for the given catalog entry. Currently every model uses the
/// same defaults; kept as a function so per-model overrides are a
/// one-line change.
pub fn config_for(_entry: &ModelEntry) -> FimConfig {
    FimConfig::default()
}

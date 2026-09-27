//! Inline AI — local llama.cpp sidecar for ghost-text SQL completion.
//!
//! See `Vibecoding/inline-ai.md` for the design doc. This module covers the
//! Phase B scope:
//!
//! * `models` — model catalog, download, delete.
//! * `sidecar` — spawn/stop/health-check `llama-server`.
//! * `state` — the application-level state holder that keeps the
//!   running sidecar handle and in-flight download map.
//!
//! Phase C (FIM completer) will land in `completer.rs` alongside these.

pub mod binaries;
pub mod completer;
pub mod fim;
pub mod models;
pub mod sidecar;
pub mod state;

//! Owned by `AppState`: tracks running training jobs so the UI can
//! report progress and cancel them.
//!
//! The inner maps live behind `Arc<Mutex<…>>` so detached background
//! tasks (`training_jobs::start` spawns one per job) can hold a cheap
//! clone without having to borrow `AppState`.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

use super::jobs::TrainingJob;

#[derive(Clone)]
pub struct TrainingState {
    /// Job records, keyed by job id. Includes running and recently-
    /// finished jobs (kept for history in the UI).
    pub jobs: Arc<Mutex<HashMap<String, TrainingJob>>>,
    /// Cancel flags for running jobs. Entry is present iff the job is
    /// still active.
    pub cancels: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
}

impl TrainingState {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            cancels: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for TrainingState {
    fn default() -> Self {
        Self::new()
    }
}

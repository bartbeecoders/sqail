//! Git integration (Phase 1).
//!
//! Wraps `git2` (libgit2) to let a user:
//!   - init/open/clone a local repo
//!   - snapshot DB schema → per-object DDL files
//!   - stage, commit, push, pull, fetch
//!   - inspect working-tree status + file diffs
//!
//! Design notes:
//!   - No in-process repo handle cache: every call opens the repo fresh from
//!     its path. libgit2 open is cheap and this keeps us lock-free and safe
//!     across tauri async workers.
//!   - Pull intentionally does NOT touch the connected database. The caller
//!     is expected to review the working-tree diff and either apply manually
//!     or use the AI migration flow.

pub mod commands;
pub mod commit;
pub mod diff;
pub mod remote;
pub mod repo;
pub mod snapshot;
pub mod status;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSignature {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCredentials {
    /// HTTPS username (leave empty to use plaintext pat in `password`).
    pub username: Option<String>,
    /// Password or personal access token.
    pub password: Option<String>,
}

impl GitCredentials {
    pub fn none() -> Self {
        Self {
            username: None,
            password: None,
        }
    }
}

pub(crate) fn map_err(e: git2::Error) -> String {
    format!("git: {}", e.message())
}

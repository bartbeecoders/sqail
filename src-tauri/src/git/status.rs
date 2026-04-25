//! Working-tree status (modified/new/deleted/staged).

use std::path::Path;

use git2::{Status, StatusOptions};
use serde::{Deserialize, Serialize};

use super::{map_err, repo::open_at};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileChange {
    pub path: String,
    /// "new" | "modified" | "deleted" | "renamed" | "typechange" | "conflicted"
    pub kind: String,
    /// `true` if the change (or part of it) is in the index.
    pub staged: bool,
    /// `true` if there are also unstaged modifications on top of a staged change.
    pub has_unstaged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResponse {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub files: Vec<GitFileChange>,
}

pub fn status(path: &Path) -> Result<GitStatusResponse, String> {
    let repo = open_at(path)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repo.statuses(Some(&mut opts)).map_err(map_err)?;

    let mut files = Vec::new();
    for entry in statuses.iter() {
        let st = entry.status();
        let path = entry.path().unwrap_or_default().to_string();
        if path.is_empty() {
            continue;
        }
        let (kind, staged, has_unstaged) = classify(st);
        files.push(GitFileChange { path, kind, staged, has_unstaged });
    }

    let (branch, upstream, ahead, behind) = branch_tracking(&repo);
    Ok(GitStatusResponse { branch, upstream, ahead, behind, files })
}

fn classify(st: Status) -> (String, bool, bool) {
    if st.contains(Status::CONFLICTED) {
        return ("conflicted".into(), false, true);
    }
    let index_changed = st.intersects(
        Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE,
    );
    let wt_changed = st.intersects(
        Status::WT_NEW
            | Status::WT_MODIFIED
            | Status::WT_DELETED
            | Status::WT_RENAMED
            | Status::WT_TYPECHANGE,
    );
    let kind = if st.intersects(Status::INDEX_NEW | Status::WT_NEW) {
        "new"
    } else if st.intersects(Status::INDEX_DELETED | Status::WT_DELETED) {
        "deleted"
    } else if st.intersects(Status::INDEX_RENAMED | Status::WT_RENAMED) {
        "renamed"
    } else if st.intersects(Status::INDEX_TYPECHANGE | Status::WT_TYPECHANGE) {
        "typechange"
    } else {
        "modified"
    };
    (kind.into(), index_changed, wt_changed && index_changed)
}

fn branch_tracking(repo: &git2::Repository) -> (Option<String>, Option<String>, usize, usize) {
    if repo.is_empty().unwrap_or(false) {
        return (None, None, 0, 0);
    }
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return (None, None, 0, 0),
    };
    let branch_name = head.shorthand().map(|s| s.to_string());
    let Some(local_oid) = head.target() else {
        return (branch_name, None, 0, 0);
    };
    let Some(branch_short) = branch_name.clone() else {
        return (branch_name, None, 0, 0);
    };
    let local_branch = match repo.find_branch(&branch_short, git2::BranchType::Local) {
        Ok(b) => b,
        Err(_) => return (branch_name, None, 0, 0),
    };
    let upstream = match local_branch.upstream() {
        Ok(u) => u,
        Err(_) => return (branch_name, None, 0, 0),
    };
    let upstream_name = upstream
        .name()
        .ok()
        .flatten()
        .map(|s| s.to_string());
    let upstream_oid = match upstream.get().target() {
        Some(oid) => oid,
        None => return (branch_name, upstream_name, 0, 0),
    };
    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid).unwrap_or((0, 0));
    (branch_name, upstream_name, ahead, behind)
}

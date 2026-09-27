//! Repository lifecycle: init, open, clone.

use std::path::{Path, PathBuf};

use git2::{Repository, RepositoryInitOptions};
use serde::{Deserialize, Serialize};

use super::{map_err, GitCredentials};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub path: String,
    pub head_branch: Option<String>,
    pub head_commit: Option<String>,
    pub is_empty: bool,
    pub remotes: Vec<String>,
}

pub fn init(path: &Path) -> Result<RepoInfo, String> {
    std::fs::create_dir_all(path).map_err(|e| format!("create dir: {e}"))?;
    let mut opts = RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = Repository::init_opts(path, &opts).map_err(map_err)?;
    describe(&repo, path)
}

pub fn open(path: &Path) -> Result<RepoInfo, String> {
    let repo = Repository::open(path).map_err(map_err)?;
    describe(&repo, path)
}

pub fn clone(url: &str, dest: &Path, creds: &GitCredentials) -> Result<RepoInfo, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let mut builder = git2::build::RepoBuilder::new();
    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.remote_callbacks(build_callbacks(creds));
    builder.fetch_options(fetch_opts);
    let repo = builder.clone(url, dest).map_err(map_err)?;
    describe(&repo, dest)
}

pub(crate) fn open_at(path: &Path) -> Result<Repository, String> {
    Repository::open(path).map_err(map_err)
}

fn describe(repo: &Repository, path: &Path) -> Result<RepoInfo, String> {
    let head_branch;
    let head_commit;
    let is_empty = repo.is_empty().unwrap_or(false);
    if is_empty {
        head_branch = None;
        head_commit = None;
    } else {
        match repo.head() {
            Ok(head_ref) => {
                head_branch = head_ref.shorthand().map(|s| s.to_string());
                head_commit = head_ref.peel_to_commit().ok().map(|c| c.id().to_string());
            }
            Err(_) => {
                head_branch = None;
                head_commit = None;
            }
        }
    }
    let remotes = repo
        .remotes()
        .map_err(map_err)?
        .iter()
        .filter_map(|r| r.map(|s| s.to_string()))
        .collect();
    Ok(RepoInfo {
        path: canonicalize_display(path),
        head_branch,
        head_commit,
        is_empty,
        remotes,
    })
}

fn canonicalize_display(path: &Path) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

pub fn list_branches(path: &Path) -> Result<Vec<String>, String> {
    let repo = open_at(path)?;
    let branches = repo.branches(Some(git2::BranchType::Local)).map_err(map_err)?;
    let mut out = Vec::new();
    for b in branches {
        let (branch, _) = b.map_err(map_err)?;
        if let Some(name) = branch.name().map_err(map_err)? {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

pub fn current_branch(path: &Path) -> Result<Option<String>, String> {
    let repo = open_at(path)?;
    if repo.is_empty().unwrap_or(false) {
        return Ok(None);
    }
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };
    Ok(head.shorthand().map(|s| s.to_string()))
}

/// Shared callback builder for anything that talks to a remote.
pub(crate) fn build_callbacks<'a>(creds: &'a GitCredentials) -> git2::RemoteCallbacks<'a> {
    let mut cb = git2::RemoteCallbacks::new();
    let u = creds.username.clone();
    let p = creds.password.clone();
    cb.credentials(move |_url, username_from_url, allowed| {
        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            let user = u.clone().or_else(|| username_from_url.map(|s| s.to_string())).unwrap_or_else(|| "git".to_string());
            let pass = p.clone().unwrap_or_default();
            return git2::Cred::userpass_plaintext(&user, &pass);
        }
        if allowed.contains(git2::CredentialType::DEFAULT) {
            return git2::Cred::default();
        }
        Err(git2::Error::from_str("no supported credential type"))
    });
    cb
}

pub fn resolve_repo_path(raw: &str) -> PathBuf {
    PathBuf::from(raw)
}

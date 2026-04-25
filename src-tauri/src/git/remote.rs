//! Remote configuration + fetch/pull/push.

use std::path::Path;

use git2::{AutotagOption, FetchOptions, PushOptions, Repository};
use serde::{Deserialize, Serialize};

use super::{
    map_err,
    repo::{build_callbacks, open_at},
    GitCredentials,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemote {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    /// "up-to-date" | "fast-forward" | "merge-required" | "no-upstream"
    pub kind: String,
    pub message: String,
    /// Changed paths after a fast-forward (relative to repo root). Empty for
    /// merge-required, since we explicitly refuse to merge here.
    pub changed_files: Vec<String>,
}

pub fn list_remotes(repo_path: &Path) -> Result<Vec<GitRemote>, String> {
    let repo = open_at(repo_path)?;
    let names = repo.remotes().map_err(map_err)?;
    let mut out = Vec::new();
    for name in names.iter().flatten() {
        let remote = repo.find_remote(name).map_err(map_err)?;
        out.push(GitRemote {
            name: name.to_string(),
            url: remote.url().unwrap_or_default().to_string(),
        });
    }
    Ok(out)
}

pub fn set_remote(repo_path: &Path, name: &str, url: &str) -> Result<GitRemote, String> {
    let repo = open_at(repo_path)?;
    if repo.find_remote(name).is_ok() {
        repo.remote_set_url(name, url).map_err(map_err)?;
    } else {
        repo.remote(name, url).map_err(map_err)?;
    }
    Ok(GitRemote {
        name: name.to_string(),
        url: url.to_string(),
    })
}

pub fn remove_remote(repo_path: &Path, name: &str) -> Result<(), String> {
    let repo = open_at(repo_path)?;
    repo.remote_delete(name).map_err(map_err)
}

pub fn fetch(
    repo_path: &Path,
    remote_name: &str,
    creds: &GitCredentials,
) -> Result<(), String> {
    let repo = open_at(repo_path)?;
    let mut remote = repo.find_remote(remote_name).map_err(map_err)?;
    let mut opts = FetchOptions::new();
    opts.remote_callbacks(build_callbacks(creds));
    opts.download_tags(AutotagOption::All);
    let refspecs: Vec<String> = remote
        .fetch_refspecs()
        .map_err(map_err)?
        .iter()
        .filter_map(|r| r.map(|s| s.to_string()))
        .collect();
    remote
        .fetch(&refspecs, Some(&mut opts), None)
        .map_err(map_err)?;
    Ok(())
}

/// Fetch + fast-forward. If a non-fast-forward merge would be required we
/// bail out and let the user reconcile — sqail does not perform silent merges
/// and (crucially) never touches the DB.
pub fn pull(
    repo_path: &Path,
    remote_name: &str,
    creds: &GitCredentials,
) -> Result<PullResult, String> {
    fetch(repo_path, remote_name, creds)?;

    let repo = open_at(repo_path)?;
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => {
            return Ok(PullResult {
                kind: "no-upstream".into(),
                message: "Repository has no HEAD yet.".into(),
                changed_files: Vec::new(),
            });
        }
    };
    let branch_short = head.shorthand().unwrap_or_default().to_string();
    let Ok(local_branch) = repo.find_branch(&branch_short, git2::BranchType::Local) else {
        return Ok(PullResult {
            kind: "no-upstream".into(),
            message: "HEAD is not a local branch.".into(),
            changed_files: Vec::new(),
        });
    };
    let Ok(upstream) = local_branch.upstream() else {
        return Ok(PullResult {
            kind: "no-upstream".into(),
            message: format!("Branch '{branch_short}' has no upstream."),
            changed_files: Vec::new(),
        });
    };
    let upstream_oid = upstream.get().target().ok_or("upstream has no target")?;
    let upstream_commit = repo
        .annotated_commit_from_fetchhead(&branch_short, "", &upstream_oid)
        .or_else(|_| repo.find_annotated_commit(upstream_oid))
        .map_err(map_err)?;

    let (analysis, _) = repo.merge_analysis(&[&upstream_commit]).map_err(map_err)?;
    if analysis.is_up_to_date() {
        return Ok(PullResult {
            kind: "up-to-date".into(),
            message: "Already up to date.".into(),
            changed_files: Vec::new(),
        });
    }
    if analysis.is_fast_forward() {
        let changed = fast_forward_changed_files(&repo, upstream_oid)?;
        let refname = format!("refs/heads/{branch_short}");
        let mut reference = repo.find_reference(&refname).map_err(map_err)?;
        reference
            .set_target(upstream_oid, "sqail: pull fast-forward")
            .map_err(map_err)?;
        repo.set_head(&refname).map_err(map_err)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(map_err)?;
        return Ok(PullResult {
            kind: "fast-forward".into(),
            message: format!("Fast-forwarded {branch_short}."),
            changed_files: changed,
        });
    }
    Ok(PullResult {
        kind: "merge-required".into(),
        message:
            "Remote has diverged from local. Resolve the merge with your git client, then reload."
                .into(),
        changed_files: Vec::new(),
    })
}

fn fast_forward_changed_files(
    repo: &Repository,
    upstream_oid: git2::Oid,
) -> Result<Vec<String>, String> {
    let head_commit = repo.head().and_then(|h| h.peel_to_commit()).map_err(map_err)?;
    let head_tree = head_commit.tree().map_err(map_err)?;
    let upstream_commit = repo.find_commit(upstream_oid).map_err(map_err)?;
    let upstream_tree = upstream_commit.tree().map_err(map_err)?;
    let diff = repo
        .diff_tree_to_tree(Some(&head_tree), Some(&upstream_tree), None)
        .map_err(map_err)?;
    let mut files = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            if let Some(p) = delta.new_file().path().or(delta.old_file().path()) {
                files.push(p.to_string_lossy().to_string());
            }
            true
        },
        None,
        None,
        None,
    )
    .map_err(map_err)?;
    Ok(files)
}

pub fn push(
    repo_path: &Path,
    remote_name: &str,
    branch: &str,
    creds: &GitCredentials,
) -> Result<(), String> {
    let repo = open_at(repo_path)?;
    let mut remote = repo.find_remote(remote_name).map_err(map_err)?;
    let mut opts = PushOptions::new();
    opts.remote_callbacks(build_callbacks(creds));
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    remote
        .push(&[&refspec], Some(&mut opts))
        .map_err(map_err)?;
    Ok(())
}

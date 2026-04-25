//! File + tree diffs.

use std::path::Path;

use git2::{DiffOptions, Repository};
use serde::{Deserialize, Serialize};

use super::{map_err, repo::open_at};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileDiff {
    pub path: String,
    /// Old file contents at HEAD (empty for new files).
    pub old_content: String,
    /// New file contents in the working tree (empty for deletions).
    pub new_content: String,
    /// Unified diff text (best-effort, may be empty for binary files).
    pub patch: String,
}

/// Diff between HEAD and the working tree for a single file. Used to render
/// the Monaco diff view when a user clicks a modified file in the status list.
pub fn file_diff_head_to_wt(repo_path: &Path, file: &str) -> Result<GitFileDiff, String> {
    let repo = open_at(repo_path)?;
    let old_content = read_head_blob(&repo, file).unwrap_or_default();
    let new_content = read_workdir_blob(&repo, file).unwrap_or_default();
    let patch = build_patch(&repo, file)?;
    Ok(GitFileDiff {
        path: file.to_string(),
        old_content,
        new_content,
        patch,
    })
}

/// Diff for every file changed since HEAD — returns the hunk text per file.
/// Used by the AI migration generator to reason over DDL deltas.
pub fn all_changed_files(repo_path: &Path) -> Result<Vec<GitFileDiff>, String> {
    let repo = open_at(repo_path)?;
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let mut opts = DiffOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let diff = repo
        .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .map_err(map_err)?;

    let mut out: Vec<GitFileDiff> = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            if let Some(p) = delta.new_file().path().or(delta.old_file().path()) {
                let s = p.to_string_lossy().to_string();
                out.push(GitFileDiff {
                    path: s,
                    old_content: String::new(),
                    new_content: String::new(),
                    patch: String::new(),
                });
            }
            true
        },
        None,
        None,
        None,
    )
    .map_err(map_err)?;

    for f in out.iter_mut() {
        f.old_content = read_head_blob(&repo, &f.path).unwrap_or_default();
        f.new_content = read_workdir_blob(&repo, &f.path).unwrap_or_default();
        f.patch = build_patch(&repo, &f.path).unwrap_or_default();
    }
    Ok(out)
}

fn read_head_blob(repo: &Repository, file: &str) -> Option<String> {
    let head = repo.head().ok()?;
    let tree = head.peel_to_tree().ok()?;
    let entry = tree.get_path(Path::new(file)).ok()?;
    let obj = entry.to_object(repo).ok()?;
    let blob = obj.as_blob()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

fn read_workdir_blob(repo: &Repository, file: &str) -> Option<String> {
    let wd = repo.workdir()?;
    let full = wd.join(file);
    std::fs::read_to_string(full).ok()
}

fn build_patch(repo: &Repository, file: &str) -> Result<String, String> {
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let mut opts = DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(file);
    let diff = repo
        .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .map_err(map_err)?;
    let mut patch = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if let Ok(text) = std::str::from_utf8(line.content()) {
            match line.origin() {
                '+' | '-' | ' ' => patch.push(line.origin()),
                _ => {}
            }
            patch.push_str(text);
        }
        true
    })
    .map_err(map_err)?;
    Ok(patch)
}

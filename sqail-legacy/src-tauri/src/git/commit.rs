//! Staging + commit helpers.

use std::path::{Path, PathBuf};

use git2::{IndexAddOption, ObjectType, Repository};

use super::{map_err, repo::open_at, GitSignature};

pub fn stage_paths(repo_path: &Path, paths: &[String]) -> Result<(), String> {
    let repo = open_at(repo_path)?;
    let mut index = repo.index().map_err(map_err)?;
    index
        .add_all(paths.iter().map(PathBuf::from), IndexAddOption::DEFAULT, None)
        .map_err(map_err)?;
    index.write().map_err(map_err)?;
    Ok(())
}

pub fn stage_all(repo_path: &Path) -> Result<(), String> {
    let repo = open_at(repo_path)?;
    let mut index = repo.index().map_err(map_err)?;
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .map_err(map_err)?;
    index.write().map_err(map_err)?;
    Ok(())
}

pub fn unstage_paths(repo_path: &Path, paths: &[String]) -> Result<(), String> {
    let repo = open_at(repo_path)?;
    if repo.is_empty().unwrap_or(false) {
        // No HEAD yet — reset from the empty tree.
        let mut index = repo.index().map_err(map_err)?;
        for p in paths {
            // `remove_path` only removes from index if present.
            let _ = index.remove_path(Path::new(p));
        }
        index.write().map_err(map_err)?;
        return Ok(());
    }
    let head = repo.head().map_err(map_err)?;
    let obj = head.peel(ObjectType::Commit).map_err(map_err)?;
    let pathspecs: Vec<&Path> = paths.iter().map(|s| Path::new(s.as_str())).collect();
    repo.reset_default(Some(&obj), pathspecs.iter()).map_err(map_err)?;
    Ok(())
}

pub fn commit(
    repo_path: &Path,
    message: &str,
    signature: Option<&GitSignature>,
) -> Result<String, String> {
    let repo = open_at(repo_path)?;
    let sig = resolve_signature(&repo, signature)?;

    let mut index = repo.index().map_err(map_err)?;
    let tree_id = index.write_tree().map_err(map_err)?;
    let tree = repo.find_tree(tree_id).map_err(map_err)?;

    let parents: Vec<git2::Commit> = match repo.head() {
        Ok(head) => vec![head.peel_to_commit().map_err(map_err)?],
        Err(_) => Vec::new(),
    };
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
        .map_err(map_err)?;
    Ok(oid.to_string())
}

fn resolve_signature<'a>(
    repo: &'a Repository,
    supplied: Option<&GitSignature>,
) -> Result<git2::Signature<'a>, String> {
    if let Some(s) = supplied {
        return git2::Signature::now(&s.name, &s.email).map_err(map_err);
    }
    // Try repo config → global config → fall back.
    if let Ok(sig) = repo.signature() {
        return Ok(sig);
    }
    git2::Signature::now("sqail", "sqail@localhost").map_err(map_err)
}

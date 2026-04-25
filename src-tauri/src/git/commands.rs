//! Tauri command surface for git integration.

use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::ai::client;
use crate::state::AppState;

use super::{
    commit as git_commit, diff as git_diff, remote as git_remote, repo, snapshot as git_snapshot,
    status as git_status, GitCredentials, GitSignature,
};

fn path(s: &str) -> PathBuf {
    repo::resolve_repo_path(s)
}

// ── lifecycle ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn git_init_repo(repo_path: String) -> Result<repo::RepoInfo, String> {
    repo::init(&path(&repo_path))
}

#[tauri::command]
pub async fn git_open_repo(repo_path: String) -> Result<repo::RepoInfo, String> {
    repo::open(&path(&repo_path))
}

#[tauri::command]
pub async fn git_clone_repo(
    url: String,
    dest: String,
    credentials: Option<GitCredentials>,
) -> Result<repo::RepoInfo, String> {
    let creds = credentials.unwrap_or_else(GitCredentials::none);
    repo::clone(&url, &path(&dest), &creds)
}

#[tauri::command]
pub async fn git_current_branch(repo_path: String) -> Result<Option<String>, String> {
    repo::current_branch(&path(&repo_path))
}

#[tauri::command]
pub async fn git_list_branches(repo_path: String) -> Result<Vec<String>, String> {
    repo::list_branches(&path(&repo_path))
}

// ── status + diff ─────────────────────────────────────────────────

#[tauri::command]
pub async fn git_status(repo_path: String) -> Result<git_status::GitStatusResponse, String> {
    git_status::status(&path(&repo_path))
}

#[tauri::command]
pub async fn git_file_diff(
    repo_path: String,
    file: String,
) -> Result<git_diff::GitFileDiff, String> {
    git_diff::file_diff_head_to_wt(&path(&repo_path), &file)
}

#[tauri::command]
pub async fn git_changed_files(repo_path: String) -> Result<Vec<git_diff::GitFileDiff>, String> {
    git_diff::all_changed_files(&path(&repo_path))
}

// ── staging + commit ──────────────────────────────────────────────

#[tauri::command]
pub async fn git_stage(repo_path: String, paths: Vec<String>) -> Result<(), String> {
    git_commit::stage_paths(&path(&repo_path), &paths)
}

#[tauri::command]
pub async fn git_stage_all(repo_path: String) -> Result<(), String> {
    git_commit::stage_all(&path(&repo_path))
}

#[tauri::command]
pub async fn git_unstage(repo_path: String, paths: Vec<String>) -> Result<(), String> {
    git_commit::unstage_paths(&path(&repo_path), &paths)
}

#[tauri::command]
pub async fn git_commit(
    repo_path: String,
    message: String,
    signature: Option<GitSignature>,
) -> Result<String, String> {
    git_commit::commit(&path(&repo_path), &message, signature.as_ref())
}

// ── remotes ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn git_list_remotes(repo_path: String) -> Result<Vec<git_remote::GitRemote>, String> {
    git_remote::list_remotes(&path(&repo_path))
}

#[tauri::command]
pub async fn git_set_remote(
    repo_path: String,
    name: String,
    url: String,
) -> Result<git_remote::GitRemote, String> {
    git_remote::set_remote(&path(&repo_path), &name, &url)
}

#[tauri::command]
pub async fn git_remove_remote(repo_path: String, name: String) -> Result<(), String> {
    git_remote::remove_remote(&path(&repo_path), &name)
}

#[tauri::command]
pub async fn git_fetch(
    repo_path: String,
    remote: String,
    credentials: Option<GitCredentials>,
) -> Result<(), String> {
    let creds = credentials.unwrap_or_else(GitCredentials::none);
    git_remote::fetch(&path(&repo_path), &remote, &creds)
}

#[tauri::command]
pub async fn git_pull(
    repo_path: String,
    remote: String,
    credentials: Option<GitCredentials>,
) -> Result<git_remote::PullResult, String> {
    let creds = credentials.unwrap_or_else(GitCredentials::none);
    git_remote::pull(&path(&repo_path), &remote, &creds)
}

#[tauri::command]
pub async fn git_push(
    repo_path: String,
    remote: String,
    branch: String,
    credentials: Option<GitCredentials>,
) -> Result<(), String> {
    let creds = credentials.unwrap_or_else(GitCredentials::none);
    git_remote::push(&path(&repo_path), &remote, &branch, &creds)
}

// ── schema snapshot ───────────────────────────────────────────────

#[tauri::command]
pub async fn git_snapshot_schema(
    state: State<'_, AppState>,
    repo_path: String,
    connection_id: String,
    include_routines: Option<bool>,
) -> Result<git_snapshot::SnapshotSummary, String> {
    let pools = state.pools.lock().await;
    let pool = pools
        .get(&connection_id)
        .ok_or_else(|| format!("No active pool for connection '{connection_id}'"))?
        .clone();
    drop(pools);
    let conns = state.connections.lock().await;
    let driver = conns
        .iter()
        .find(|c| c.id == connection_id)
        .map(|c| c.driver.clone())
        .ok_or_else(|| format!("Connection '{connection_id}' not found"))?;
    drop(conns);

    git_snapshot::snapshot_to_repo(
        &path(&repo_path),
        pool,
        &driver,
        include_routines.unwrap_or(true),
    )
    .await
}

// ── AI migration generation ───────────────────────────────────────

#[tauri::command]
pub async fn ai_generate_migration_script(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    diff_text: String,
    driver: String,
    provider_id: Option<String>,
) -> Result<String, String> {
    let config = crate::commands::get_provider(&state, provider_id).await?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let rid = request_id.clone();
    let user_msg = format!("Schema diff (multiple files, unified diff format):\n\n{diff_text}");
    tokio::spawn(async move {
        client::stream_ai_response(
            app_handle,
            rid,
            &config,
            &user_msg,
            "generate_migration",
            Some(&driver),
            None,
        )
        .await;
    });
    Ok(request_id)
}

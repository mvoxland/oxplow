use oxplow_domain::stores::StreamStore;
use oxplow_git::{
    AheadBehind, BlameLine, BranchChanges, ChangeScopes, CommitRefLabel, GitOpResult,
    GitOperationKind, GitWorktreeEntry, GroupedGitRefs, LocalBlameEntry, RemoteBranchEntry,
    RepoConflictState, TextSearchHit,
};
use std::collections::HashMap;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_repo_conflict_state(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<RepoConflictState, IpcError> {
    Ok(state.git.conflict_state(stream_id.as_deref()).await)
}

#[tauri::command]
#[specta::specta]
pub async fn get_ahead_behind(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base: String,
    head: String,
) -> Result<AheadBehind, IpcError> {
    Ok(state
        .git
        .ahead_behind(stream_id.as_deref(), base, head)
        .await)
}

#[tauri::command]
#[specta::specta]
pub async fn append_to_gitignore(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    entry: String,
) -> Result<(), IpcError> {
    state
        .git
        .append_to_gitignore(stream_id.as_deref(), entry)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn restore_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
) -> Result<(), IpcError> {
    state
        .git
        .restore_path(stream_id.as_deref(), path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

/// Re-export the operation kind so the TS bindings include it.
pub fn _capture_git_operation_kind() -> GitOperationKind {
    GitOperationKind::Merge
}

#[tauri::command]
#[specta::specta]
pub async fn git_fetch(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: Option<String>,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .fetch(stream_id.as_deref(), remote)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .pull(stream_id.as_deref())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull_remote_into_current(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .pull_remote_into_current(stream_id.as_deref(), remote, branch)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_push(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .push(stream_id.as_deref())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_push_current_to(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .push_current_to(stream_id.as_deref(), remote, branch)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_merge_into(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    source: String,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .merge(stream_id.as_deref(), source)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_rebase_onto(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    onto: String,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .rebase(stream_id.as_deref(), onto)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_commit_all(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    message: String,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .commit_all(stream_id.as_deref(), message)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn git_add_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
) -> Result<GitOpResult, IpcError> {
    state
        .git
        .add_path(stream_id.as_deref(), path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_all_refs(state: tauri::State<'_, AppState>) -> Result<GroupedGitRefs, IpcError> {
    Ok(state.git.list_all_refs().await)
}

/// Map commit SHAs to a single user-facing branch/tag label. Used by
/// the Local History dashboard to chip each parent snapshot with its
/// pinned commit's branch/tag name; SHAs that match no ref are absent
/// from the result (caller renders a short-sha fallback).
#[tauri::command]
#[specta::specta]
pub async fn resolve_commit_ref_labels(
    state: tauri::State<'_, AppState>,
    shas: Vec<String>,
) -> Result<HashMap<String, Vec<CommitRefLabel>>, IpcError> {
    Ok(state.git.resolve_commit_ref_labels(shas).await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_remote_branches(
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<RemoteBranchEntry>, IpcError> {
    Ok(state
        .git
        .list_recent_remote_branches(limit.unwrap_or(50))
        .await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_file_commits(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
    limit: Option<usize>,
) -> Result<Vec<oxplow_git::GitLogCommit>, IpcError> {
    Ok(state
        .git
        .list_file_commits(stream_id.as_deref(), path, limit.unwrap_or(50))
        .await)
}

#[tauri::command]
#[specta::specta]
pub async fn git_blame(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
) -> Result<Vec<BlameLine>, IpcError> {
    Ok(state.git.blame(stream_id.as_deref(), path).await)
}

#[tauri::command]
#[specta::specta]
pub async fn local_blame(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
    disk_text: String,
) -> Result<Vec<LocalBlameEntry>, IpcError> {
    Ok(state
        .git
        .local_blame(stream_id.as_deref(), path, disk_text)
        .await)
}

#[tauri::command]
#[specta::specta]
pub async fn get_change_scopes(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<ChangeScopes, IpcError> {
    Ok(state.git.change_scopes(stream_id.as_deref()).await)
}

#[tauri::command]
#[specta::specta]
pub async fn get_branch_changes(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base_ref: String,
) -> Result<BranchChanges, IpcError> {
    Ok(state
        .git
        .branch_changes(stream_id.as_deref(), base_ref)
        .await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_existing_worktrees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    Ok(state.git.list_existing_worktrees().await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_adoptable_worktrees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    let store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    let registered: Vec<String> = store
        .list()
        .await?
        .into_iter()
        .map(|s| s.worktree_path)
        .collect();
    Ok(state.git.list_adoptable_worktrees(registered).await)
}

#[tauri::command]
#[specta::specta]
pub async fn search_workspace_text(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<TextSearchHit>, IpcError> {
    Ok(state
        .git
        .search_workspace_text(stream_id.as_deref(), query, limit)
        .await)
}

#[tauri::command]
#[specta::specta]
pub async fn read_file_at_ref(
    state: tauri::State<'_, AppState>,
    r#ref: String,
    path: String,
) -> Result<Option<String>, IpcError> {
    Ok(state.git.read_file_at_ref(r#ref, path).await)
}

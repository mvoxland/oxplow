use oxplow_git::{
    AheadBehind, BlameLine, BranchChanges, GitOpResult, GitOperationKind, GitWorktreeEntry,
    GroupedGitRefs, RemoteBranchEntry, RepoConflictState, TextSearchHit,
};
use oxplow_domain::stores::StreamStore;

use crate::error::IpcError;
use crate::state::AppState;

fn project_dir(state: &tauri::State<'_, AppState>) -> std::path::PathBuf {
    state.layout.project_dir.clone()
}

async fn run_blocking_io<R>(
    f: impl FnOnce() -> std::io::Result<R> + Send + 'static,
) -> Result<R, IpcError>
where
    R: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn get_repo_conflict_state(
    state: tauri::State<'_, AppState>,
) -> Result<RepoConflictState, IpcError> {
    let path = state.layout.project_dir.clone();
    let s = tokio::task::spawn_blocking(move || oxplow_git::get_repo_conflict_state(&path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(s)
}

#[tauri::command]
#[specta::specta]
pub async fn get_ahead_behind(
    state: tauri::State<'_, AppState>,
    base: String,
    head: String,
) -> Result<AheadBehind, IpcError> {
    let path = state.layout.project_dir.clone();
    let ab = tokio::task::spawn_blocking(move || oxplow_git::get_ahead_behind(&path, &base, &head))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(ab)
}

#[tauri::command]
#[specta::specta]
pub async fn append_to_gitignore(
    state: tauri::State<'_, AppState>,
    entry: String,
) -> Result<(), IpcError> {
    let path = state.layout.project_dir.clone();
    tokio::task::spawn_blocking(move || oxplow_git::append_to_gitignore(&path, &entry))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn restore_path(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), IpcError> {
    let project = state.layout.project_dir.clone();
    tokio::task::spawn_blocking(move || oxplow_git::restore_path(&project, &path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}

/// Re-export the operation kind so the TS bindings include it.
pub fn _capture_git_operation_kind() -> GitOperationKind {
    GitOperationKind::Merge
}

#[tauri::command]
#[specta::specta]
pub async fn git_fetch(
    state: tauri::State<'_, AppState>,
    remote: Option<String>,
) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::fetch(&path, remote.as_deref())).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull(state: tauri::State<'_, AppState>) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::pull(&path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull_remote_into_current(
    state: tauri::State<'_, AppState>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::pull_remote_into_current(&path, &remote, &branch)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_push(state: tauri::State<'_, AppState>) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::push(&path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_push_current_to(
    state: tauri::State<'_, AppState>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::push_current_to(&path, &remote, &branch)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_merge_into(
    state: tauri::State<'_, AppState>,
    source: String,
) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::merge(&path, &source)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_rebase_onto(
    state: tauri::State<'_, AppState>,
    onto: String,
) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::rebase(&path, &onto)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_commit_all(
    state: tauri::State<'_, AppState>,
    message: String,
) -> Result<GitOpResult, IpcError> {
    let path = project_dir(&state);
    run_blocking_io(move || oxplow_git::commit_all(&path, &message)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_add_path(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<GitOpResult, IpcError> {
    let project = project_dir(&state);
    run_blocking_io(move || oxplow_git::add_path(&project, &path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_all_refs(
    state: tauri::State<'_, AppState>,
) -> Result<GroupedGitRefs, IpcError> {
    let path = project_dir(&state);
    Ok(tokio::task::spawn_blocking(move || oxplow_git::list_all_refs(&path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_remote_branches(
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<RemoteBranchEntry>, IpcError> {
    let path = project_dir(&state);
    let limit = limit.unwrap_or(50);
    Ok(
        tokio::task::spawn_blocking(move || oxplow_git::list_recent_remote_branches(&path, limit))
            .await
            .map_err(|e| IpcError::internal(e.to_string()))?,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn list_file_commits(
    state: tauri::State<'_, AppState>,
    path: String,
    limit: Option<usize>,
) -> Result<Vec<oxplow_git::GitLogCommit>, IpcError> {
    let project = project_dir(&state);
    let limit = limit.unwrap_or(50);
    Ok(
        tokio::task::spawn_blocking(move || oxplow_git::list_file_commits(&project, &path, limit))
            .await
            .map_err(|e| IpcError::internal(e.to_string()))?,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn git_blame(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<BlameLine>, IpcError> {
    let project = project_dir(&state);
    Ok(tokio::task::spawn_blocking(move || oxplow_git::git_blame(&project, &path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_branch_changes(
    state: tauri::State<'_, AppState>,
    base_ref: String,
) -> Result<BranchChanges, IpcError> {
    let project = project_dir(&state);
    Ok(tokio::task::spawn_blocking(move || oxplow_git::list_branch_changes(&project, &base_ref))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_existing_worktrees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    let path = project_dir(&state);
    Ok(tokio::task::spawn_blocking(move || oxplow_git::list_existing_worktrees(&path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_sibling_worktrees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    let path = project_dir(&state);
    Ok(tokio::task::spawn_blocking(move || oxplow_git::list_sibling_worktrees(&path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_adoptable_worktrees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GitWorktreeEntry>, IpcError> {
    let path = project_dir(&state);
    let store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    let registered: Vec<String> = store
        .list()
        .await?
        .into_iter()
        .map(|s| s.worktree_path)
        .collect();
    Ok(tokio::task::spawn_blocking(move || {
        oxplow_git::list_adoptable_worktrees(&path, &registered)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn search_workspace_text(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<TextSearchHit>, IpcError> {
    let project = project_dir(&state);
    Ok(
        tokio::task::spawn_blocking(move || oxplow_git::search_workspace_text(&project, &query, limit))
            .await
            .map_err(|e| IpcError::internal(e.to_string()))?,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn read_file_at_ref(
    state: tauri::State<'_, AppState>,
    r#ref: String,
    path: String,
) -> Result<Option<String>, IpcError> {
    let project = project_dir(&state);
    Ok(tokio::task::spawn_blocking(move || oxplow_git::read_file_at_ref(&project, &r#ref, &path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

use oxplow_git::{
    AheadBehind, BlameLine, BranchChanges, ChangeScopes, GitOpResult, GitOperationKind,
    GitWorktreeEntry, GroupedGitRefs, LocalBlameEntry, RemoteBranchEntry, RepoConflictState,
    TextSearchHit,
};
use oxplow_domain::stores::StreamStore;

use crate::error::IpcError;
use crate::state::AppState;

fn project_dir(state: &tauri::State<'_, AppState>) -> std::path::PathBuf {
    state.layout.project_dir.clone()
}

/// Resolve the working directory a git op should run against. When
/// `stream_id` is provided and matches a registered stream, the
/// stream's worktree path wins. Otherwise we fall back to the
/// project root, matching the pre-per-stream behavior. Worktree
/// paths recorded as relative are resolved against the project dir.
///
/// Public so other command modules (`log`, `snapshot`, …) can reuse
/// it without duplicating the lookup.
pub(crate) async fn resolve_repo_dir(
    state: &tauri::State<'_, AppState>,
    stream_id: Option<&str>,
) -> std::path::PathBuf {
    let Some(id) = stream_id else {
        return project_dir(state);
    };
    let store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    if let Ok(streams) = store.list().await {
        if let Some(s) = streams.into_iter().find(|s| s.id.as_str() == id) {
            let raw = std::path::PathBuf::from(&s.worktree_path);
            if raw.is_absolute() {
                return raw;
            }
            return state.layout.project_dir.join(raw);
        }
    }
    project_dir(state)
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
    stream_id: Option<String>,
) -> Result<RepoConflictState, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    let s = tokio::task::spawn_blocking(move || oxplow_git::get_repo_conflict_state(&path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(s)
}

#[tauri::command]
#[specta::specta]
pub async fn get_ahead_behind(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base: String,
    head: String,
) -> Result<AheadBehind, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    let ab = tokio::task::spawn_blocking(move || oxplow_git::get_ahead_behind(&path, &base, &head))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(ab)
}

#[tauri::command]
#[specta::specta]
pub async fn append_to_gitignore(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    entry: String,
) -> Result<(), IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
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
    stream_id: Option<String>,
    path: String,
) -> Result<(), IpcError> {
    let project = resolve_repo_dir(&state, stream_id.as_deref()).await;
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
    stream_id: Option<String>,
    remote: Option<String>,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::fetch(&path, remote.as_deref())).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::pull(&path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_pull_remote_into_current(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::pull_remote_into_current(&path, &remote, &branch)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_push(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::push(&path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_push_current_to(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    remote: String,
    branch: String,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::push_current_to(&path, &remote, &branch)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_merge_into(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    source: String,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::merge(&path, &source)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_rebase_onto(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    onto: String,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::rebase(&path, &onto)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_commit_all(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    message: String,
) -> Result<GitOpResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    run_blocking_io(move || oxplow_git::commit_all(&path, &message)).await
}

#[tauri::command]
#[specta::specta]
pub async fn git_add_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
) -> Result<GitOpResult, IpcError> {
    let project = resolve_repo_dir(&state, stream_id.as_deref()).await;
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
    stream_id: Option<String>,
    path: String,
    limit: Option<usize>,
) -> Result<Vec<oxplow_git::GitLogCommit>, IpcError> {
    let project = resolve_repo_dir(&state, stream_id.as_deref()).await;
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
    stream_id: Option<String>,
    path: String,
) -> Result<Vec<BlameLine>, IpcError> {
    let project = resolve_repo_dir(&state, stream_id.as_deref()).await;
    Ok(tokio::task::spawn_blocking(move || oxplow_git::git_blame(&project, &path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn local_blame(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    path: String,
    disk_text: String,
) -> Result<Vec<LocalBlameEntry>, IpcError> {
    let project = resolve_repo_dir(&state, stream_id.as_deref()).await;
    Ok(tokio::task::spawn_blocking(move || {
        oxplow_git::local_blame(&project, &path, &disk_text)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_change_scopes(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<ChangeScopes, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    Ok(tokio::task::spawn_blocking(move || oxplow_git::get_change_scopes(&path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_branch_changes(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base_ref: String,
) -> Result<BranchChanges, IpcError> {
    let project = resolve_repo_dir(&state, stream_id.as_deref()).await;
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
    stream_id: Option<String>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<TextSearchHit>, IpcError> {
    let project = resolve_repo_dir(&state, stream_id.as_deref()).await;
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

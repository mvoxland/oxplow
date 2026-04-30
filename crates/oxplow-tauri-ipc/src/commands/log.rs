use oxplow_git::{CommitDetail, GitLogCommit, GitLogOptions, GitLogResult};

use crate::commands::git::resolve_repo_dir;
use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_git_log(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    limit: Option<u32>,
    all: bool,
) -> Result<GitLogResult, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    let opts = GitLogOptions {
        limit: limit.map(|n| n as usize),
        all,
    };
    let result = tokio::task::spawn_blocking(move || oxplow_git::get_git_log(&path, opts))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn get_commit_detail(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    sha: String,
) -> Result<Option<CommitDetail>, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    let detail = tokio::task::spawn_blocking(move || oxplow_git::get_commit_detail(&path, &sha))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(detail)
}

#[tauri::command]
#[specta::specta]
pub async fn get_commits_ahead_of(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base: String,
    head: String,
    limit: u32,
) -> Result<Vec<GitLogCommit>, IpcError> {
    let path = resolve_repo_dir(&state, stream_id.as_deref()).await;
    let commits = tokio::task::spawn_blocking(move || {
        oxplow_git::get_commits_ahead_of(&path, &base, &head, limit as usize)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(commits)
}

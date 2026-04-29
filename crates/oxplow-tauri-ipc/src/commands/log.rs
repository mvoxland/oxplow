use oxplow_git::{CommitDetail, GitLogCommit, GitLogOptions, GitLogResult};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_git_log(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
    all: bool,
) -> Result<GitLogResult, IpcError> {
    let path = state.layout.project_dir.clone();
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
    sha: String,
) -> Result<Option<CommitDetail>, IpcError> {
    let path = state.layout.project_dir.clone();
    let detail = tokio::task::spawn_blocking(move || oxplow_git::get_commit_detail(&path, &sha))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(detail)
}

#[tauri::command]
#[specta::specta]
pub async fn get_commits_ahead_of(
    state: tauri::State<'_, AppState>,
    base: String,
    head: String,
    limit: u32,
) -> Result<Vec<GitLogCommit>, IpcError> {
    let path = state.layout.project_dir.clone();
    let commits = tokio::task::spawn_blocking(move || {
        oxplow_git::get_commits_ahead_of(&path, &base, &head, limit as usize)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(commits)
}

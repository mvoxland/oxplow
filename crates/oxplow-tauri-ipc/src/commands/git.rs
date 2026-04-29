use oxplow_git::{AheadBehind, GitOperationKind, RepoConflictState};

use crate::error::IpcError;
use crate::state::AppState;

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

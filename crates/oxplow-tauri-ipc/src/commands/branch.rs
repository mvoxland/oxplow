use oxplow_git::{BranchRef, BranchRefKind};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_branches(state: tauri::State<'_, AppState>) -> Result<Vec<BranchRef>, IpcError> {
    Ok(state.git.list_branches_project().await)
}

#[tauri::command]
#[specta::specta]
pub async fn get_default_branch(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, IpcError> {
    Ok(state.git.detect_default_branch().await)
}

#[tauri::command]
#[specta::specta]
pub async fn rename_branch(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<(), IpcError> {
    state
        .git
        .rename_branch(from, to)
        .await
        .map_err(|e| IpcError::invalid(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_branch(
    state: tauri::State<'_, AppState>,
    branch: String,
    force: bool,
) -> Result<(), IpcError> {
    state
        .git
        .delete_branch(branch, force)
        .await
        .map_err(|e| IpcError::invalid(e.to_string()))
}

/// Filter helper for the UI that wants only locals or only remotes.
#[tauri::command]
#[specta::specta]
pub async fn list_local_branches(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<BranchRef>, IpcError> {
    let all = state.git.list_branches_project().await;
    Ok(all
        .into_iter()
        .filter(|b| b.kind == BranchRefKind::Local)
        .collect())
}

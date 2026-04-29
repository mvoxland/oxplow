use oxplow_git::{BranchRef, BranchRefKind};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_branches(state: tauri::State<'_, AppState>) -> Result<Vec<BranchRef>, IpcError> {
    let path = state.layout.project_dir.clone();
    let branches = tokio::task::spawn_blocking(move || oxplow_git::list_branches(path))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(branches)
}

#[tauri::command]
#[specta::specta]
pub async fn get_default_branch(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, IpcError> {
    let path = state.layout.project_dir.clone();
    let detected =
        tokio::task::spawn_blocking(move || oxplow_git::detect_default_branch(&path))
            .await
            .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(detected)
}

#[tauri::command]
#[specta::specta]
pub async fn rename_branch(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<(), IpcError> {
    let path = state.layout.project_dir.clone();
    tokio::task::spawn_blocking(move || oxplow_git::rename_branch(&path, &from, &to))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::invalid(e.to_string()))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_branch(
    state: tauri::State<'_, AppState>,
    branch: String,
    force: bool,
) -> Result<(), IpcError> {
    let path = state.layout.project_dir.clone();
    tokio::task::spawn_blocking(move || oxplow_git::delete_branch(&path, &branch, force))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::invalid(e.to_string()))?;
    Ok(())
}

/// Filter helper for the UI that wants only locals or only remotes.
#[tauri::command]
#[specta::specta]
pub async fn list_local_branches(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<BranchRef>, IpcError> {
    let path = state.layout.project_dir.clone();
    let branches = tokio::task::spawn_blocking(move || {
        oxplow_git::list_branches(path)
            .into_iter()
            .filter(|b| b.kind == BranchRefKind::Local)
            .collect()
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(branches)
}

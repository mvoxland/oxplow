//! Concrete `#[tauri::command]` functions.
//!
//! Each command is a thin adapter: extract the `AppState`, call into
//! `oxplow-app`, convert errors at the boundary into `IpcError`. The
//! UI's TS bindings are generated from these via `tauri-specta`.

use specta::Type;
use serde::{Deserialize, Serialize};

use oxplow_domain::stores::{ThreadStore, WorkItemStore};
use oxplow_domain::{Stream, StreamId, Thread, ThreadId, WorkItem};

use crate::error::IpcError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppVersion {
    pub version: &'static str,
}

#[tauri::command]
#[specta::specta]
pub async fn app_version() -> Result<AppVersion, IpcError> {
    Ok(AppVersion {
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn list_streams(state: tauri::State<'_, AppState>) -> Result<Vec<Stream>, IpcError> {
    state
        .streams
        .list_streams()
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn ensure_primary(state: tauri::State<'_, AppState>) -> Result<Stream, IpcError> {
    state
        .streams
        .ensure_primary()
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateWorktreeRequest {
    pub slug: String,
    pub title: String,
    pub branch: String,
    #[serde(rename = "branchSource")]
    pub branch_source: String,
}

#[tauri::command]
#[specta::specta]
pub async fn create_worktree(
    state: tauri::State<'_, AppState>,
    req: CreateWorktreeRequest,
) -> Result<Stream, IpcError> {
    state
        .streams
        .create_worktree(&req.slug, req.title, req.branch, req.branch_source)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_stream(
    state: tauri::State<'_, AppState>,
    id: StreamId,
) -> Result<(), IpcError> {
    state
        .streams
        .delete_stream(&id)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_threads(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    state
        .thread_store
        .list_for_stream(&stream_id)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_work_items_for_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<WorkItem>, IpcError> {
    state
        .work_item_store
        .list_for_thread(&thread_id)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_backlog(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WorkItem>, IpcError> {
    state
        .work_item_store
        .list_backlog()
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

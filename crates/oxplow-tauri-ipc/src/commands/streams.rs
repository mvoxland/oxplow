use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::OxplowEvent;
use oxplow_domain::{Stream, StreamId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_streams(state: tauri::State<'_, AppState>) -> Result<Vec<Stream>, IpcError> {
    Ok(state.streams.list_streams().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn ensure_primary(state: tauri::State<'_, AppState>) -> Result<Stream, IpcError> {
    Ok(state.streams.ensure_primary().await?)
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
    let stream = state
        .streams
        .create_worktree(&req.slug, req.title, req.branch, req.branch_source)
        .await?;
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(stream)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_stream(
    state: tauri::State<'_, AppState>,
    id: StreamId,
) -> Result<(), IpcError> {
    state.streams.delete_stream(&id).await?;
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(())
}

/// Returns the primary stream — the project root. Useful for any UI
/// path that needs to know "what does the user think of as 'this'
/// project?" without enumerating the full list.
#[tauri::command]
#[specta::specta]
pub async fn get_primary_stream(
    state: tauri::State<'_, AppState>,
) -> Result<Option<Stream>, IpcError> {
    use oxplow_domain::stores::StreamStore;
    let stream_store = oxplow_db::SqliteStreamStore::new(state.db.clone());
    Ok(stream_store.primary().await?)
}

/// Currently-selected stream (None falls back to primary in the UI).
#[tauri::command]
#[specta::specta]
pub async fn get_current_stream(
    state: tauri::State<'_, AppState>,
) -> Result<Option<Stream>, IpcError> {
    Ok(state.streams.current().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn switch_stream(
    state: tauri::State<'_, AppState>,
    id: Option<StreamId>,
) -> Result<(), IpcError> {
    state.streams.set_current(id.as_ref()).await?;
    state
        .events
        .emit(OxplowEvent::CurrentStreamChanged { stream_id: id });
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenameStreamRequest {
    pub id: StreamId,
    pub title: String,
}

#[tauri::command]
#[specta::specta]
pub async fn rename_stream(
    state: tauri::State<'_, AppState>,
    req: RenameStreamRequest,
) -> Result<Stream, IpcError> {
    let s = state.streams.rename(&req.id, req.title).await?;
    state.events.emit(OxplowEvent::StreamsChanged);
    Ok(s)
}

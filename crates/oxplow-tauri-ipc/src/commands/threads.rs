use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{StreamId, Thread, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_threads(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    Ok(state.thread_store.list_for_stream(&stream_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Option<Thread>, IpcError> {
    Ok(state.thread_store.get(&thread_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_thread(
    state: tauri::State<'_, AppState>,
    thread: Thread,
) -> Result<(), IpcError> {
    Ok(state.thread_store.upsert(&thread).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<(), IpcError> {
    Ok(state.thread_store.delete(&thread_id).await?)
}

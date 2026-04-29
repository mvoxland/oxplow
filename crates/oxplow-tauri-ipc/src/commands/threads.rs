use serde::{Deserialize, Serialize};
use specta::Type;

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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateThreadRequest {
    #[serde(rename = "streamId")]
    pub stream_id: StreamId,
    pub title: String,
    #[serde(rename = "paneTarget")]
    pub pane_target: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn create_thread(
    state: tauri::State<'_, AppState>,
    req: CreateThreadRequest,
) -> Result<Thread, IpcError> {
    let pane = req.pane_target.unwrap_or_else(|| "working".into());
    Ok(state
        .threads
        .create(&req.stream_id, req.title, pane)
        .await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenameThreadRequest {
    pub id: ThreadId,
    pub title: String,
}

#[tauri::command]
#[specta::specta]
pub async fn rename_thread(
    state: tauri::State<'_, AppState>,
    req: RenameThreadRequest,
) -> Result<Thread, IpcError> {
    Ok(state.threads.rename(&req.id, req.title).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SetThreadPromptRequest {
    pub id: ThreadId,
    pub prompt: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn set_thread_prompt(
    state: tauri::State<'_, AppState>,
    req: SetThreadPromptRequest,
) -> Result<Thread, IpcError> {
    Ok(state.threads.set_prompt(&req.id, req.prompt).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn promote_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    Ok(state.threads.promote(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn close_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    Ok(state.threads.close(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn reopen_thread(
    state: tauri::State<'_, AppState>,
    id: ThreadId,
) -> Result<Thread, IpcError> {
    Ok(state.threads.reopen(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_closed_threads(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Vec<Thread>, IpcError> {
    Ok(state.threads.list_closed(&stream_id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReorderThreadQueueRequest {
    #[serde(rename = "streamId")]
    pub stream_id: StreamId,
    pub order: Vec<ThreadId>,
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_thread_queue(
    state: tauri::State<'_, AppState>,
    req: ReorderThreadQueueRequest,
) -> Result<(), IpcError> {
    state
        .threads
        .reorder_queue(&req.stream_id, &req.order)
        .await?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_selected_thread(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
) -> Result<Option<ThreadId>, IpcError> {
    Ok(state.threads.selected(&stream_id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SelectThreadRequest {
    #[serde(rename = "streamId")]
    pub stream_id: StreamId,
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
}

#[tauri::command]
#[specta::specta]
pub async fn select_thread(
    state: tauri::State<'_, AppState>,
    req: SelectThreadRequest,
) -> Result<(), IpcError> {
    state
        .threads
        .select(&req.stream_id, req.thread_id.as_ref())
        .await?;
    Ok(())
}

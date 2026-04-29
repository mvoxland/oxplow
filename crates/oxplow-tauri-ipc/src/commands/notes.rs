//! Work-item / thread notes (the in-app comment thread on a work
//! item or the thread-scoped capture pad).

use oxplow_domain::stores::{WorkItemEventStore, WorkNoteStore};
use oxplow_domain::{NoteId, ThreadId, WorkItemEvent, WorkItemId, WorkNote};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn add_work_note(
    state: tauri::State<'_, AppState>,
    work_item_id: WorkItemId,
    body: String,
    author: String,
) -> Result<WorkNote, IpcError> {
    Ok(state
        .work_note_store
        .add_for_item(&work_item_id, &body, &author)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn add_thread_note(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
    body: String,
    author: String,
) -> Result<WorkNote, IpcError> {
    Ok(state
        .work_note_store
        .add_for_thread(&thread_id, &body, &author)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_work_notes(
    state: tauri::State<'_, AppState>,
    work_item_id: WorkItemId,
) -> Result<Vec<WorkNote>, IpcError> {
    Ok(state.work_note_store.list_for_item(&work_item_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_thread_notes(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<WorkNote>, IpcError> {
    Ok(state.work_note_store.list_for_thread(&thread_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_work_note(
    state: tauri::State<'_, AppState>,
    id: NoteId,
) -> Result<(), IpcError> {
    Ok(state.work_note_store.delete(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_work_item_events(
    state: tauri::State<'_, AppState>,
    item_id: Option<WorkItemId>,
    thread_id: Option<ThreadId>,
) -> Result<Vec<WorkItemEvent>, IpcError> {
    match (item_id, thread_id) {
        (Some(i), _) => Ok(state.work_item_event_store.list_for_item(&i).await?),
        (None, Some(t)) => Ok(state.work_item_event_store.list_for_thread(&t).await?),
        (None, None) => Ok(vec![]),
    }
}

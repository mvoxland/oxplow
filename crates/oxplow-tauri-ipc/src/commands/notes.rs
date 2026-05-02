//! Thread-scoped notes (the per-thread capture pad backing the
//! Explore-subagent findings flow). Per-work-item notes were retired
//! — work_item_effort.summary already records what shipped on a
//! task, so a separate note table for the same purpose was duplicative.

use oxplow_domain::stores::{WorkItemEventStore, WorkNoteStore};
use oxplow_domain::{NoteId, ThreadId, WorkItemEvent, WorkItemId, WorkNote};

use crate::error::IpcError;
use crate::state::AppState;

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

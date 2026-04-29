use oxplow_app::OxplowEvent;
use oxplow_domain::stores::WorkItemStore;
use oxplow_domain::{ThreadId, WorkItem, WorkItemId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_work_items_for_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<WorkItem>, IpcError> {
    Ok(state.work_item_store.list_for_thread(&thread_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_work_item(
    state: tauri::State<'_, AppState>,
    id: WorkItemId,
) -> Result<Option<WorkItem>, IpcError> {
    Ok(state.work_item_store.get(&id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_work_item(
    state: tauri::State<'_, AppState>,
    item: WorkItem,
) -> Result<(), IpcError> {
    let thread_id = item.thread_id.clone();
    state.work_item_store.upsert(&item).await?;
    state
        .events
        .emit(OxplowEvent::WorkItemsChanged { thread_id });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_work_item(
    state: tauri::State<'_, AppState>,
    id: WorkItemId,
) -> Result<(), IpcError> {
    let thread_id = state.work_item_store.get(&id).await?.and_then(|i| i.thread_id);
    state.work_item_store.soft_delete(&id).await?;
    state
        .events
        .emit(OxplowEvent::WorkItemsChanged { thread_id });
    Ok(())
}

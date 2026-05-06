use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::{CreateWorkItemInput, OxplowEvent, UpdateWorkItemChanges};
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
    let thread_id = state
        .work_item_store
        .get(&id)
        .await?
        .and_then(|i| i.thread_id);
    state.work_item_store.soft_delete(&id).await?;
    state
        .events
        .emit(OxplowEvent::WorkItemsChanged { thread_id });
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateWorkItemRequest {
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
    pub input: CreateWorkItemInput,
}

#[tauri::command]
#[specta::specta]
pub async fn create_work_item(
    state: tauri::State<'_, AppState>,
    req: CreateWorkItemRequest,
) -> Result<WorkItem, IpcError> {
    let item = state
        .work_items
        .create(req.thread_id.clone(), req.input)
        .await?;
    state.events.emit(OxplowEvent::WorkItemsChanged {
        thread_id: req.thread_id,
    });
    Ok(item)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateWorkItemRequest {
    pub id: WorkItemId,
    pub changes: UpdateWorkItemChanges,
}

#[tauri::command]
#[specta::specta]
pub async fn update_work_item(
    state: tauri::State<'_, AppState>,
    req: UpdateWorkItemRequest,
) -> Result<WorkItem, IpcError> {
    let item = state.work_items.update(&req.id, req.changes).await?;
    state.events.emit(OxplowEvent::WorkItemsChanged {
        thread_id: item.thread_id.clone(),
    });
    Ok(item)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReorderWorkItemsRequest {
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
    pub order: Vec<WorkItemId>,
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_work_items(
    state: tauri::State<'_, AppState>,
    req: ReorderWorkItemsRequest,
) -> Result<(), IpcError> {
    state
        .work_items
        .reorder(req.thread_id.as_ref(), &req.order)
        .await?;
    state.events.emit(OxplowEvent::WorkItemsChanged {
        thread_id: req.thread_id,
    });
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MoveWorkItemRequest {
    pub id: WorkItemId,
    /// Destination thread, or `None` to move onto the backlog.
    #[serde(rename = "threadId")]
    pub thread_id: Option<ThreadId>,
}

#[tauri::command]
#[specta::specta]
pub async fn get_work_item_summaries(
    state: tauri::State<'_, AppState>,
    thread_id: Option<ThreadId>,
) -> Result<Vec<WorkItem>, IpcError> {
    Ok(match thread_id {
        Some(t) => state.work_item_store.list_for_thread(&t).await?,
        None => state.work_item_store.list_backlog().await?,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn move_work_item(
    state: tauri::State<'_, AppState>,
    req: MoveWorkItemRequest,
) -> Result<WorkItem, IpcError> {
    // Capture origin so the renderer can refetch both buckets.
    let origin_thread_id = state
        .work_item_store
        .get(&req.id)
        .await?
        .and_then(|i| i.thread_id);
    let item = state
        .work_items
        .move_to(&req.id, req.thread_id.clone())
        .await?;
    state.events.emit(OxplowEvent::WorkItemsChanged {
        thread_id: origin_thread_id,
    });
    state.events.emit(OxplowEvent::WorkItemsChanged {
        thread_id: req.thread_id,
    });
    Ok(item)
}

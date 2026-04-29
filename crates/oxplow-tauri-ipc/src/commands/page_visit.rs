use oxplow_db::analytics_stores::PageVisitStore as _;
use oxplow_db::PageVisit;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VisitedPage {
    pub page_kind: String,
    pub page_id: String,
    pub visit_count: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn record_page_visit(
    state: tauri::State<'_, AppState>,
    page_kind: String,
    page_id: String,
    duration_ms: Option<i64>,
) -> Result<PageVisit, IpcError> {
    Ok(state
        .page_visit_store
        .record(&page_kind, &page_id, duration_ms)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_page_visits(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<PageVisit>, IpcError> {
    Ok(state.page_visit_store.list_recent(limit as usize).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn top_visited_pages(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<VisitedPage>, IpcError> {
    let pairs = state.page_visit_store.list_top(limit as usize).await?;
    Ok(pairs
        .into_iter()
        .map(|(page_kind, page_id, visit_count)| VisitedPage {
            page_kind,
            page_id,
            visit_count,
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn forget_page(
    state: tauri::State<'_, AppState>,
    page_kind: String,
    page_id: String,
) -> Result<(), IpcError> {
    Ok(state
        .page_visit_store
        .forget_page(&page_kind, &page_id)
        .await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PageVisitDay {
    pub day: String,
    pub count: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn count_page_visits_by_day(
    state: tauri::State<'_, AppState>,
    days: u32,
) -> Result<Vec<PageVisitDay>, IpcError> {
    let rows = state.page_visit_store.count_by_day(days).await?;
    Ok(rows
        .into_iter()
        .map(|(day, count)| PageVisitDay { day, count })
        .collect())
}

use oxplow_app::OxplowEvent;
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
    thread_id: Option<String>,
) -> Result<PageVisit, IpcError> {
    let visit = state
        .page_visit_store
        .record(&page_kind, &page_id, duration_ms, thread_id.as_deref())
        .await?;
    state.events.emit(OxplowEvent::PageVisitChanged);
    Ok(visit)
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_page_visits(
    state: tauri::State<'_, AppState>,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<PageVisit>, IpcError> {
    Ok(state
        .page_visit_store
        .list_recent(limit as usize, thread_id.as_deref())
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn top_visited_pages(
    state: tauri::State<'_, AppState>,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<VisitedPage>, IpcError> {
    let pairs = state
        .page_visit_store
        .list_top(limit as usize, thread_id.as_deref())
        .await?;
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
    state
        .page_visit_store
        .forget_page(&page_kind, &page_id)
        .await?;
    state.events.emit(OxplowEvent::PageVisitChanged);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PageVisitDay {
    pub day: String,
    pub count: i64,
}

#[tauri::command]
#[specta::specta]
pub async fn list_frequent_usage(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<PageVisit>, IpcError> {
    Ok(state.page_visit_store.list_frequent(limit as usize).await?)
}

/// Pages currently kept open in editor tabs (best-effort: derived from
/// recent visits whose duration_ms is null — i.e. the open-event hasn't
/// been closed yet). The renderer already filters to its own tab list.
#[tauri::command]
#[specta::specta]
pub async fn list_currently_open_usage(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<PageVisit>, IpcError> {
    let recent = state.page_visit_store.list_recent(limit as usize * 4, None).await?;
    Ok(recent
        .into_iter()
        .filter(|v| v.duration_ms.is_none())
        .take(limit as usize)
        .collect())
}

/// Pages whose latest visit has a duration_ms set (i.e. the editor
/// closed them). Drives the "recently finished" rail.
#[tauri::command]
#[specta::specta]
pub async fn list_recently_finished(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<PageVisit>, IpcError> {
    let recent = state.page_visit_store.list_recent(limit as usize * 4, None).await?;
    Ok(recent
        .into_iter()
        .filter(|v| v.duration_ms.is_some())
        .take(limit as usize)
        .collect())
}

/// Drop the duration_ms-bearing rows so they stop appearing in
/// "recently finished".
#[tauri::command]
#[specta::specta]
pub async fn clear_recently_finished(
    state: tauri::State<'_, AppState>,
) -> Result<(), IpcError> {
    let recent = state.page_visit_store.list_recent(10_000, None).await?;
    let mut changed = false;
    for v in recent.into_iter().filter(|v| v.duration_ms.is_some()) {
        state
            .page_visit_store
            .forget_page(&v.page_kind, &v.page_id)
            .await?;
        changed = true;
    }
    if changed {
        state.events.emit(OxplowEvent::PageVisitChanged);
    }
    Ok(())
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

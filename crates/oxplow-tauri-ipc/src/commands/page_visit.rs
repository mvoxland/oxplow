use oxplow_app::OxplowEvent;
use oxplow_db::analytics_stores::PageVisitStore as _;
use oxplow_db::PageVisit;
use oxplow_domain::stores::WorkItemStore as _;
use oxplow_domain::{ThreadId, Timestamp, WorkItemStatus};
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
    let recent = state
        .page_visit_store
        .list_recent(limit as usize * 4, None)
        .await?;
    Ok(recent
        .into_iter()
        .filter(|v| v.duration_ms.is_none())
        .take(limit as usize)
        .collect())
}

/// Recently completed work items merged with recently updated wiki
/// notes, sorted by timestamp DESC. Drives the rail's "Finished"
/// section. Items whose timestamp is `<= finished_cleared_at` are
/// hidden until something newer lands.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FinishedEntry {
    #[serde(rename = "work-item")]
    WorkItem {
        #[serde(rename = "itemId")]
        item_id: String,
        title: String,
        t: Timestamp,
    },
    #[serde(rename = "note")]
    Note {
        slug: String,
        title: String,
        t: Timestamp,
    },
}

impl FinishedEntry {
    fn timestamp(&self) -> Timestamp {
        match self {
            FinishedEntry::WorkItem { t, .. } => *t,
            FinishedEntry::Note { t, .. } => *t,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_recently_finished(
    state: tauri::State<'_, AppState>,
    thread_id: Option<String>,
    limit: u32,
) -> Result<Vec<FinishedEntry>, IpcError> {
    let cap = limit.max(1) as usize;
    let cursor_key = thread_id.clone().unwrap_or_default();
    let cleared_at = state
        .finished_cleared_at
        .read()
        .expect("finished_cleared_at rwlock")
        .get(&cursor_key)
        .copied();

    let mut entries: Vec<FinishedEntry> = Vec::new();

    if let Some(tid) = thread_id.as_ref() {
        // Thread-scoped: only items filed against this thread, only
        // wiki pages the thread actually touched.
        let tid = ThreadId::from(tid.clone());
        let items = state.work_item_store.list_for_thread(&tid).await?;
        for item in items {
            if item.status != WorkItemStatus::Done {
                continue;
            }
            let Some(t) = item.completed_at else { continue };
            entries.push(FinishedEntry::WorkItem {
                item_id: item.id.0,
                title: item.title,
                t,
            });
        }
        let touches = state
            .wiki_page_thread_updates
            .list_for_thread(&tid, cap * 4)
            .await?;
        for touch in touches {
            let Some(note) = state.wiki_page_store.get(&touch.slug).await? else {
                continue;
            };
            entries.push(FinishedEntry::Note {
                slug: note.slug,
                title: note.title,
                // Use the per-thread timestamp so a different thread
                // editing the same note doesn't promote this thread's
                // entry.
                t: touch.last_seen_at,
            });
        }
    } else {
        // No thread context — fall back to a global view (used for
        // initial paint before a thread is selected).
        let done = state.work_item_store.list_recently_done(cap).await?;
        for item in done {
            let Some(t) = item.completed_at else { continue };
            entries.push(FinishedEntry::WorkItem {
                item_id: item.id.0,
                title: item.title,
                t,
            });
        }
        let notes = state.wiki_page_store.list().await?;
        for note in notes.into_iter().take(cap) {
            entries.push(FinishedEntry::Note {
                slug: note.slug,
                title: note.title,
                t: note.updated_at,
            });
        }
    }

    entries.retain(|e| match cleared_at {
        Some(cursor) => e.timestamp() > cursor,
        None => true,
    });
    entries.sort_by(|a, b| b.timestamp().cmp(&a.timestamp()));
    entries.truncate(cap);
    Ok(entries)
}

/// Hide the current "Finished" entries behind a cursor. Source rows
/// (work items / wiki pages) are untouched; new finishes still surface
/// because their timestamp is newer than the cursor. Cursor is
/// per-thread so clearing one thread's section doesn't blank another.
#[tauri::command]
#[specta::specta]
pub async fn clear_recently_finished(
    state: tauri::State<'_, AppState>,
    thread_id: Option<String>,
) -> Result<(), IpcError> {
    let key = thread_id.unwrap_or_default();
    state
        .finished_cleared_at
        .write()
        .expect("finished_cleared_at rwlock")
        .insert(key, Timestamp::now());
    state.events.emit(OxplowEvent::PageVisitChanged);
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

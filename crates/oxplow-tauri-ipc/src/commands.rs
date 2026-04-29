//! Concrete `#[tauri::command]` functions.
//!
//! Each command is a thin adapter: extract the `AppState`, call into
//! `oxplow-app`, convert errors at the boundary into `IpcError`. The
//! UI's TS bindings are generated from these via `tauri-specta`.

use specta::Type;
use serde::{Deserialize, Serialize};

use oxplow_domain::stores::{ThreadStore, WorkItemStore};
use oxplow_domain::{Stream, StreamId, Thread, ThreadId, WorkItem};

use crate::error::IpcError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppVersion {
    pub version: &'static str,
}

#[tauri::command]
#[specta::specta]
pub async fn app_version() -> Result<AppVersion, IpcError> {
    Ok(AppVersion {
        version: env!("CARGO_PKG_VERSION"),
    })
}

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
    Ok(state
        .streams
        .create_worktree(&req.slug, req.title, req.branch, req.branch_source)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_stream(
    state: tauri::State<'_, AppState>,
    id: StreamId,
) -> Result<(), IpcError> {
    Ok(state.streams.delete_stream(&id).await?)
}

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
pub async fn list_work_items_for_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<WorkItem>, IpcError> {
    Ok(state.work_item_store.list_for_thread(&thread_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_backlog(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WorkItem>, IpcError> {
    Ok(state.work_item_store.list_backlog().await?)
}

/// Open an external URL in a sandboxed `WebviewWindow`.
///
/// Replaces the legacy `<webview>` tag flow. The new window inherits
/// the `external-url` capability defined in
/// `apps/desktop/src-tauri/capabilities/external-url.json`, which
/// grants zero oxplow commands and zero plugin permissions — the
/// embedded content can't call back into the host.
///
/// `url` must be `http(s)://`. Anything else returns an `INVALID`
/// IpcError; the UI is expected to validate before calling, but the
/// Rust side enforces the invariant since the URL ultimately controls
/// what the new webview loads.
#[tauri::command]
#[specta::specta]
pub async fn open_external_url(
    app: tauri::AppHandle,
    url: String,
) -> Result<String, IpcError> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(IpcError::invalid(format!(
            "external URL must be http or https: {url}"
        )));
    }

    let parsed = tauri::Url::parse(&url)
        .map_err(|e| IpcError::invalid(format!("bad URL: {e}")))?;

    // Label format must match the `ext-url-*` glob in
    // capabilities/external-url.json — that's what restricts the new
    // webview to the empty-permission scope.
    let label = format!("ext-url-{}", uuid::Uuid::new_v4().simple());

    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::External(parsed),
    )
    .title(format!("{} — Oxplow", url))
    .inner_size(1100.0, 800.0)
    .build()
    .map_err(|e| IpcError::internal(format!("create webview window: {e}")))?;

    Ok(label)
}

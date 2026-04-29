use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_db::FileSnapshot;
use oxplow_domain::StreamId;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_snapshots(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<FileSnapshot>, IpcError> {
    Ok(state.snapshot_store.list_for_path(&path).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_snapshots_for_stream(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<FileSnapshot>, IpcError> {
    Ok(state
        .snapshot_store
        .list_for_stream(stream_id.as_str(), limit.unwrap_or(200))
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_snapshot(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<Option<FileSnapshot>, IpcError> {
    Ok(state.snapshot_store.get(id).await?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SnapshotPairDiff {
    pub before: Option<FileSnapshot>,
    pub after: Option<FileSnapshot>,
    /// True when the two captures hash differently (i.e. content
    /// changed between them). Always false when either side is None.
    pub changed: bool,
}

/// Compare two captures of the same path. The renderer surfaces this
/// in the snapshots panel as "what changed between then and now".
#[tauri::command]
#[specta::specta]
pub async fn get_snapshot_pair_diff(
    state: tauri::State<'_, AppState>,
    before_id: Option<i64>,
    after_id: Option<i64>,
) -> Result<SnapshotPairDiff, IpcError> {
    let before = match before_id {
        Some(id) => state.snapshot_store.get(id).await?,
        None => None,
    };
    let after = match after_id {
        Some(id) => state.snapshot_store.get(id).await?,
        None => None,
    };
    let changed = match (&before, &after) {
        (Some(b), Some(a)) => b.blob_hash != a.blob_hash,
        _ => false,
    };
    Ok(SnapshotPairDiff {
        before,
        after,
        changed,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SnapshotSummary {
    pub stream_id: Option<StreamId>,
    /// Most recent capture per path; capped at `limit` (default 200).
    pub latest: Vec<FileSnapshot>,
    pub total_captured: i64,
}

/// Most recent N captures for a stream, with a total count. Used by
/// the snapshots-panel header.
#[tauri::command]
#[specta::specta]
pub async fn get_snapshot_summary(
    state: tauri::State<'_, AppState>,
    stream_id: Option<StreamId>,
    limit: Option<usize>,
) -> Result<SnapshotSummary, IpcError> {
    let limit = limit.unwrap_or(200);
    let latest = match &stream_id {
        Some(s) => state.snapshot_store.list_for_stream(s.as_str(), limit).await?,
        None => vec![],
    };
    let total_captured = latest.len() as i64;
    Ok(SnapshotSummary {
        stream_id,
        latest,
        total_captured,
    })
}

/// Restore a file's contents from a snapshot. Best-effort: snapshots
/// only store the blob hash, not the bytes — so the actual restore
/// requires content-addressed blob storage that the new schema doesn't
/// model yet. This command currently returns an error if the snapshot
/// exists but no blob is available.
#[tauri::command]
#[specta::specta]
pub async fn restore_file_from_snapshot(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<(), IpcError> {
    let snap = state
        .snapshot_store
        .get(snapshot_id)
        .await?
        .ok_or_else(IpcError::not_found)?;
    Err(IpcError::internal(format!(
        "blob restore not yet implemented (snapshot {} hash={:?})",
        snap.id, snap.blob_hash
    )))
}

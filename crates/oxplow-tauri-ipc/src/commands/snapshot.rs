use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_db::{FileSnapshot, Snapshot, SnapshotChangeEntry, SnapshotStats};
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
pub async fn list_file_snapshots_for_stream(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<FileSnapshot>, IpcError> {
    Ok(state
        .snapshot_store
        .list_for_stream(stream_id.as_str(), limit.unwrap_or(200))
        .await?)
}

/// `snapshot` rows for a stream — one entry per `request_snapshot()`
/// call that captured anything. Newest first.
#[tauri::command]
#[specta::specta]
pub async fn list_snapshots_for_stream(
    state: tauri::State<'_, AppState>,
    stream_id: StreamId,
    limit: Option<usize>,
) -> Result<Vec<Snapshot>, IpcError> {
    Ok(state
        .snapshot_store
        .list_snapshots_for_stream(stream_id.as_str(), limit.unwrap_or(200))
        .await?)
}

/// Created/modified/deleted counts for a snapshot. Powers the Local
/// History dashboard's per-snapshot stats column.
#[tauri::command]
#[specta::specta]
pub async fn get_snapshot_stats(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<SnapshotStats, IpcError> {
    Ok(state.snapshot_store.stats_for_snapshot(snapshot_id).await?)
}

/// Per-file change entries for one snapshot, in the shape the
/// renderer's `useSnapshotChangeAnalysis` hook expects so it can
/// feed the same SummaryCard / ChangeAnalysisPanel components the
/// Git pages use.
#[tauri::command]
#[specta::specta]
pub async fn list_snapshot_change_entries(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<Vec<SnapshotChangeEntry>, IpcError> {
    Ok(state
        .snapshot_store
        .list_changes_for_snapshot(snapshot_id)
        .await?)
}

/// Read a `file_snapshot` row's blob content as a UTF-8 string.
/// Returns `None` when:
/// - the row id doesn't exist,
/// - the row has no blob hash (deletion row or oversize-tracked),
/// - the blob has been pruned from disk.
///
/// Binary bytes pass through as UTF-8 lossy — the renderer's diff /
/// function-analysis pipeline treats the result as text either way.
#[tauri::command]
#[specta::specta]
pub async fn read_snapshot_file_content(
    state: tauri::State<'_, AppState>,
    file_snapshot_id: i64,
) -> Result<Option<String>, IpcError> {
    let Some(snap) = state.snapshot_store.get(file_snapshot_id).await? else {
        return Ok(None);
    };
    let Some(hash) = snap.blob_hash.clone() else {
        return Ok(None);
    };
    let blobs = state.blobs.clone();
    let bytes = tokio::task::spawn_blocking(move || blobs.read(&hash))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    match bytes {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(_) => Ok(None),
    }
}

/// Total on-disk size of every blob in the content-addressed store.
/// Used by the Local History dashboard's Storage card.
#[tauri::command]
#[specta::specta]
pub async fn get_blob_storage_bytes(state: tauri::State<'_, AppState>) -> Result<i64, IpcError> {
    let blobs = state.blobs.clone();
    let total = tokio::task::spawn_blocking(move || blobs.total_bytes())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(total as i64)
}

/// For each snapshot id in the input list, the wiki slugs whose
/// body changed in that snapshot. Drives the Local History
/// dashboard's wiki badges. Cheaper than fetching the full
/// `file_snapshot` rows per snapshot.
#[tauri::command]
#[specta::specta]
pub async fn list_wiki_slugs_for_snapshots(
    state: tauri::State<'_, AppState>,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<(i64, String)>, IpcError> {
    Ok(state
        .snapshot_store
        .list_wiki_slugs_for_snapshots(snapshot_ids)
        .await?)
}

/// Every `file_snapshot` row captured under a single parent
/// snapshot id (i.e. one batch of `request_snapshot()`).
#[tauri::command]
#[specta::specta]
pub async fn list_files_for_snapshot(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<Vec<FileSnapshot>, IpcError> {
    Ok(state
        .snapshot_store
        .list_files_for_snapshot(snapshot_id)
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
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntry {
    pub hash: String,
    pub mtime_ms: i64,
    pub size: i64,
    /// "present" for normal captures, "oversize" for files that
    /// exceeded the configured cap (no blob written).
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SnapshotFileRow {
    pub entry: SnapshotEntry,
    /// "created" when this is the first capture of `path`,
    /// "updated" when the prior capture had a different blob,
    /// "deleted" when the current capture has no blob (file gone).
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Default)]
pub struct SnapshotSummaryCounts {
    pub created: i64,
    pub updated: i64,
    pub deleted: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    pub snapshot: FileSnapshot,
    pub previous_snapshot_id: Option<String>,
    pub files: std::collections::HashMap<String, SnapshotFileRow>,
    pub counts: SnapshotSummaryCounts,
}

/// Build a per-snapshot summary: the FileSnapshot row, the id of the
/// prior capture of the same path (if any), and a one-row diff
/// describing how the captured file relates to its predecessor
/// (created / updated / deleted). The renderer's local-history pane
/// keys off this shape.
#[tauri::command]
#[specta::specta]
pub async fn get_snapshot_summary(
    state: tauri::State<'_, AppState>,
    snapshot_id: i64,
) -> Result<Option<SnapshotSummary>, IpcError> {
    let Some(snap) = state.snapshot_store.get(snapshot_id).await? else {
        return Ok(None);
    };
    // Order is DESC by captured_at; find the row immediately after
    // ours (i.e. older). Equal-timestamp ties fall back to id order
    // implicitly via SQLite's row order.
    let history = state.snapshot_store.list_for_path(&snap.path).await?;
    let mut prev: Option<&FileSnapshot> = None;
    let mut found_self = false;
    for row in &history {
        if found_self {
            prev = Some(row);
            break;
        }
        if row.id == snap.id {
            found_self = true;
        }
    }
    let kind = match (&snap.blob_hash, prev.and_then(|p| p.blob_hash.clone())) {
        (None, _) => "deleted",
        (Some(_), None) => "created",
        (Some(cur), Some(prev_hash)) if *cur == prev_hash => "updated",
        (Some(_), Some(_)) => "updated",
    };
    let state_label = if snap.oversize { "oversize" } else { "present" };
    let entry = SnapshotEntry {
        hash: snap.blob_hash.clone().unwrap_or_default(),
        mtime_ms: 0,
        size: snap.size_bytes,
        state: state_label.to_string(),
    };
    let mut files = std::collections::HashMap::new();
    files.insert(
        snap.path.clone(),
        SnapshotFileRow {
            entry,
            kind: kind.to_string(),
        },
    );
    let counts = SnapshotSummaryCounts {
        created: if kind == "created" { 1 } else { 0 },
        updated: if kind == "updated" { 1 } else { 0 },
        deleted: if kind == "deleted" { 1 } else { 0 },
    };
    Ok(Some(SnapshotSummary {
        snapshot: snap,
        previous_snapshot_id: prev.map(|p| p.id.to_string()),
        files,
        counts,
    }))
}

/// Restore a file's contents from a snapshot. Reads the bytes from
/// the content-addressed blob store using the snapshot's `blob_hash`
/// and writes them back to the snapshot's path inside the workspace.
/// Errors with NOT_FOUND if the snapshot row is gone or its blob
/// was pruned.
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
    let hash = snap
        .blob_hash
        .clone()
        .ok_or_else(|| IpcError::invalid("snapshot has no blob (oversize or pre-blob-store)"))?;
    let bytes = state
        .blobs
        .read(&hash)
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let target = state.layout.project_dir.join(&snap.path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| IpcError::internal(e.to_string()))?;
    }
    std::fs::write(&target, &bytes).map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}

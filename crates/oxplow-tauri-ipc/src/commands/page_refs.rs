//! Unified cross-page reference graph reader.
//!
//! Both directions of the edge are exposed:
//! - `list_backlinks(target_kind, target_id)` — what points AT this
//!   page. Drives the Backlinks dropdown / panel for every page kind.
//! - `list_outbound(source_kind, source_id)` — what this page points
//!   to. Drives the new Outbound dropdown.
//!
//! The reader joins source labels (wiki title, work-item title,
//! commit subject) at read time so the renderer doesn't need to do
//! a second round-trip per row. Labels are best-effort — when the
//! source is gone (e.g. a deleted work item) the label is `None`
//! and the renderer falls back to `source_id`.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_db::PageRefEdge;

use crate::error::IpcError;
use crate::state::AppState;

/// Edge plus a best-effort renderer label for the source.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BacklinkEdge {
    pub source_kind: String,
    pub source_id: String,
    pub target_kind: String,
    pub target_id: String,
    pub ref_type: String,
    pub source_extra: Option<String>,
    /// Human label for the source (wiki title, work-item title,
    /// commit subject, …). Falls back to `source_id` in the
    /// renderer when None.
    pub source_label: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn list_backlinks(
    state: tauri::State<'_, AppState>,
    target_kind: String,
    target_id: String,
    limit: Option<i64>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    let edges = state
        .page_ref_store
        .list_backlinks(&target_kind, &target_id, limit)
        .await?;
    Ok(decorate_with_labels(&state, edges).await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_outbound(
    state: tauri::State<'_, AppState>,
    source_kind: String,
    source_id: String,
    limit: Option<i64>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    let edges = state
        .page_ref_store
        .list_outbound(&source_kind, &source_id, limit)
        .await?;
    // For outbound, the "label" we want is for the *target*. We
    // keep the same struct shape, but populate `source_label` with
    // the target's label so the renderer can be kind-agnostic.
    let mut out = decorate_outbound_targets(&state, edges).await;
    // Also fold in the source label for completeness (some
    // renderers display "from <X>" too).
    let augmented = decorate_with_labels(&state, edge_views(&out)).await;
    for (i, e) in augmented.into_iter().enumerate() {
        if let Some(orig) = out.get_mut(i) {
            // If target-label was None, fall through to source label.
            if orig.source_label.is_none() {
                orig.source_label = e.source_label;
            }
        }
    }
    Ok(out)
}

fn edge_views(rows: &[BacklinkEdge]) -> Vec<PageRefEdge> {
    rows.iter()
        .map(|r| PageRefEdge {
            source_kind: r.source_kind.clone(),
            source_id: r.source_id.clone(),
            target_kind: r.target_kind.clone(),
            target_id: r.target_id.clone(),
            ref_type: r.ref_type.clone(),
            source_extra: r.source_extra.clone(),
        })
        .collect()
}

async fn decorate_with_labels(state: &AppState, edges: Vec<PageRefEdge>) -> Vec<BacklinkEdge> {
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        let label = source_label(state, &e.source_kind, &e.source_id).await;
        out.push(BacklinkEdge {
            source_kind: e.source_kind,
            source_id: e.source_id,
            target_kind: e.target_kind,
            target_id: e.target_id,
            ref_type: e.ref_type,
            source_extra: e.source_extra,
            source_label: label,
        });
    }
    out
}

async fn decorate_outbound_targets(state: &AppState, edges: Vec<PageRefEdge>) -> Vec<BacklinkEdge> {
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        let label = source_label(state, &e.target_kind, &e.target_id).await;
        out.push(BacklinkEdge {
            source_kind: e.source_kind,
            source_id: e.source_id,
            target_kind: e.target_kind,
            target_id: e.target_id,
            ref_type: e.ref_type,
            source_extra: e.source_extra,
            source_label: label,
        });
    }
    out
}

/// Best-effort label lookup by kind. Returns `None` when the row
/// doesn't exist (deleted) or the kind doesn't carry a meaningful
/// label (e.g. files use the path itself).
async fn source_label(state: &AppState, kind: &str, id: &str) -> Option<String> {
    match kind {
        "wiki" => state
            .wiki_page_store
            .get(id)
            .await
            .ok()
            .flatten()
            .map(|p| p.title),
        "work-item" => {
            use oxplow_domain::stores::WorkItemStore as _;
            state
                .work_item_store
                .get(&oxplow_domain::WorkItemId::from(id))
                .await
                .ok()
                .flatten()
                .map(|wi| wi.title)
        }
        "git-commit" => {
            // The commit detail lookup is sync (libgit2) — wrap in
            // spawn_blocking to keep us off the runtime thread.
            let repo = state.layout.project_dir.clone();
            let id = id.to_string();
            tokio::task::spawn_blocking(move || {
                oxplow_git::log::get_commit_detail(&repo, &id).map(|d| d.subject)
            })
            .await
            .ok()
            .flatten()
        }
        _ => None,
    }
}

//! One-shot boot-time backfill for the unified page-ref graph.
//!
//! The writers in `oxplow-db` mirror their outbound refs into
//! `page_ref` on every save, but pre-existing data (rows that
//! existed before the migration ran) doesn't get touched until
//! someone re-saves it. This module re-projects every relevant
//! row exactly once on boot so backlinks for an upgraded DB
//! aren't blank until the user starts editing.
//!
//! Ordering doesn't matter — projections are per-source and each
//! writer owns its own slice. The backfill is idempotent: running
//! it again replaces the same rows it wrote last time.
//!
//! Wiki bodies and recent commits are NOT re-projected here —
//! the wiki watcher's initial scan and the commit indexer's boot
//! pass already hit those paths. This module only covers the
//! kinds whose data lives entirely in SQLite (work-items, links,
//! efforts, findings).

use std::sync::Arc;

use oxplow_db::page_ref_projections::{
    effort_ref_types, effort_touched_file_edges, finding_edges, link_edge,
    work_item_body_ref_types, work_item_edges, work_item_link_ref_types, KIND_FINDING,
    KIND_WORK_ITEM,
};
use oxplow_db::WorkItemEffortStore as _;
use oxplow_db::{
    SqliteCodeQualityStore, SqlitePageRefStore, SqliteWorkItemEffortStore, SqliteWorkItemLinkStore,
    SqliteWorkItemStore,
};
use oxplow_domain::stores::WorkItemLinkStore as _;

/// Counts of rows touched per kind. Logged at INFO so the boot
/// trail makes the backfill observable.
#[derive(Debug, Default)]
pub struct BackfillCounts {
    pub work_items: usize,
    pub links: usize,
    pub efforts: usize,
    pub findings: usize,
}

/// Project every existing row into `page_ref`. Idempotent.
pub async fn run(
    page_refs: Arc<SqlitePageRefStore>,
    work_items: Arc<SqliteWorkItemStore>,
    links: Arc<SqliteWorkItemLinkStore>,
    efforts: Arc<SqliteWorkItemEffortStore>,
    findings_store: Arc<SqliteCodeQualityStore>,
) -> BackfillCounts {
    let mut counts = BackfillCounts::default();

    // 1. Work-item body slice + touched-file slice.
    if let Ok(items) = work_items.list_all_for_backfill().await {
        for item in items {
            let edges = work_item_edges(&item);
            if let Err(e) = page_refs
                .replace_source_for_ref_types(
                    KIND_WORK_ITEM,
                    item.id.as_str(),
                    work_item_body_ref_types(),
                    edges,
                )
                .await
            {
                tracing::warn!(?e, id = %item.id, "page-ref backfill: work item failed");
                continue;
            }
            counts.work_items += 1;
            // Touched-file union pulled from the effort store.
            let mut paths: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            if let Ok(item_efforts) = efforts.list_for_item(&item.id).await {
                for ef in item_efforts {
                    if let Ok(rows) = efforts.list_files(&ef.id).await {
                        for r in rows {
                            paths.insert(r.path);
                        }
                    }
                }
            }
            let path_vec: Vec<String> = paths.into_iter().collect();
            let edges = effort_touched_file_edges(item.id.as_str(), &path_vec);
            let _ = page_refs
                .replace_source_for_ref_types(
                    KIND_WORK_ITEM,
                    item.id.as_str(),
                    effort_ref_types(),
                    edges,
                )
                .await;
            if !path_vec.is_empty() {
                counts.efforts += 1;
            }
        }
    }

    // 2. Link slice — re-project the union of outgoing links per
    //    distinct from-item. (Each link contributes one edge; we
    //    write the whole slice owned by the source in one shot so
    //    deletions on the live path stay clean too.)
    if let Ok(from_items) = links.list_distinct_from_items().await {
        for from in from_items {
            let outgoing = match links.list_outgoing(&from).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let edges: Vec<_> = outgoing.iter().map(link_edge).collect();
            if let Err(e) = page_refs
                .replace_source_for_ref_types(
                    KIND_WORK_ITEM,
                    from.as_str(),
                    work_item_link_ref_types(),
                    edges,
                )
                .await
            {
                tracing::warn!(?e, id = %from, "page-ref backfill: link slice failed");
                continue;
            }
            counts.links += 1;
        }
    }

    // 3. Findings — one edge per row.
    if let Ok(rows) = findings_store.list_all_findings_for_backfill().await {
        for (id, path) in rows {
            let id_str = id.to_string();
            let edges = finding_edges(&id_str, &path);
            if page_refs
                .replace_source(KIND_FINDING, &id_str, edges)
                .await
                .is_ok()
            {
                counts.findings += 1;
            }
        }
    }

    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use oxplow_domain::stores::{StreamStore, ThreadStore, WorkItemStore};
    use oxplow_domain::{
        Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp, WorkItem,
        WorkItemActorKind, WorkItemAuthor, WorkItemId, WorkItemKind, WorkItemPriority,
        WorkItemStatus,
    };

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    #[tokio::test]
    async fn backfill_picks_up_pre_existing_work_item_refs() {
        let db = Database::in_memory();

        // Construct stores WITHOUT page_refs first so writes don't
        // mirror — this simulates pre-migration data.
        let streams = oxplow_db::SqliteStreamStore::new(db.clone());
        let threads = oxplow_db::SqliteThreadStore::new(db.clone());
        let bare_items = SqliteWorkItemStore::new(db.clone());

        streams
            .upsert(&Stream {
                id: StreamId::from("s-1"),
                kind: StreamKind::Primary,
                title: "x".into(),
                branch: "main".into(),
                branch_ref: "refs/heads/main".into(),
                branch_source: "main".into(),
                worktree_path: "/r".into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: ts(),
                updated_at: ts(),
                archived_at: None,
            })
            .await
            .unwrap();
        threads
            .upsert(&Thread {
                id: ThreadId::from("b-1"),
                stream_id: StreamId::from("s-1"),
                title: "x".into(),
                status: ThreadStatus::Active,
                sort_index: 0,
                pane_target: "working".into(),
                resume_session_id: String::new(),
                summary: String::new(),
                summary_updated_at: None,
                closed_at: None,
                custom_prompt: None,
                created_at: ts(),
                updated_at: ts(),
                archived_at: None,
            })
            .await
            .unwrap();
        bare_items
            .upsert(&WorkItem {
                id: WorkItemId::from("wi-9"),
                thread_id: Some(ThreadId::from("b-1")),
                parent_id: None,
                kind: WorkItemKind::Task,
                title: "fix".into(),
                description: "see [[src/app.rs]] and blocks wi-019zzz-2".into(),
                acceptance_criteria: None,
                status: WorkItemStatus::Ready,
                priority: WorkItemPriority::Medium,
                sort_index: 0,
                created_by: WorkItemActorKind::User,
                created_at: ts(),
                updated_at: ts(),
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(WorkItemAuthor::User),
                category: None,
                tags: None,
            })
            .await
            .unwrap();

        let page_refs = Arc::new(SqlitePageRefStore::new(db.clone()));
        // No backlinks for the file yet — writer was bare.
        let pre = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(pre.is_empty());

        // Build the attached stores the backfill consumes.
        let items_attached =
            Arc::new(SqliteWorkItemStore::new(db.clone()).with_page_refs((*page_refs).clone()));
        let links =
            Arc::new(SqliteWorkItemLinkStore::new(db.clone()).with_page_refs((*page_refs).clone()));
        let efforts = Arc::new(
            SqliteWorkItemEffortStore::new(db.clone()).with_page_refs((*page_refs).clone()),
        );
        let findings_store =
            Arc::new(SqliteCodeQualityStore::new(db.clone()).with_page_refs((*page_refs).clone()));

        let counts = run(
            page_refs.clone(),
            items_attached,
            links,
            efforts,
            findings_store,
        )
        .await;
        assert!(counts.work_items >= 1);

        let post = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert_eq!(post.len(), 1, "got {post:?}");
        assert_eq!(post[0].source_id, "wi-9");
    }
}

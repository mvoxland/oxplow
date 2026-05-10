//! SQLite-backed unified cross-page reference graph.
//!
//! Each row is one directed edge `(source) --ref_type--> (target)`.
//! Per-subsystem writers own their `source_kind` rows (the wiki sync
//! owns all `wiki` rows, the work-item save path owns all
//! `work-item` rows, …). The standard write pattern is
//! [`SqlitePageRefStore::replace_source`]: delete every row whose
//! source matches, then re-insert the new edge set in one
//! transaction. The reader joins on `(target_kind, target_id)` for
//! backlinks or `(source_kind, source_id)` for outbound — both
//! covered by indexes.
//!
//! `kind` is denormalised next to `id` so kind-filtered lookups
//! ("all backlinks where target is a file") don't need a LIKE on a
//! synthetic combined column. Canonical ids match the frontend's
//! `TabRef.id` shape (e.g. `"wiki:architecture"`, `"wi-42"`,
//! `"file:src/app.rs"`, `"git-commit:abc123"`).

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::DomainError;

use crate::database::Database;

/// One directed edge in the page graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PageRefEdge {
    pub source_kind: String,
    pub source_id: String,
    pub target_kind: String,
    pub target_id: String,
    pub ref_type: String,
    /// Optional JSON blob with edge-specific extras (line anchor,
    /// version pin, link sub-type, …). Opaque to the store.
    pub source_extra: Option<String>,
}

impl PageRefEdge {
    pub fn new(
        source_kind: impl Into<String>,
        source_id: impl Into<String>,
        target_kind: impl Into<String>,
        target_id: impl Into<String>,
        ref_type: impl Into<String>,
    ) -> Self {
        Self {
            source_kind: source_kind.into(),
            source_id: source_id.into(),
            target_kind: target_kind.into(),
            target_id: target_id.into(),
            ref_type: ref_type.into(),
            source_extra: None,
        }
    }

    pub fn with_extra(mut self, extra: impl Into<String>) -> Self {
        self.source_extra = Some(extra.into());
        self
    }
}

#[derive(Clone)]
pub struct SqlitePageRefStore {
    db: Database,
}

impl SqlitePageRefStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Replace every edge owned by `(source_kind, source_id)` with
    /// `edges`. Atomic. Idempotent: passing the same `edges` twice
    /// leaves the table unchanged. Edges in `edges` whose source
    /// doesn't match `(source_kind, source_id)` are silently skipped
    /// — callers shouldn't construct mixed batches but the store
    /// stays defensive.
    pub async fn replace_source(
        &self,
        source_kind: &str,
        source_id: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = db
                .conn()
                .map_err(|e| DomainError::Invalid(format!("pool: {e}")))?;
            let tx = conn
                .transaction()
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            tx.execute(
                "DELETE FROM page_ref WHERE source_kind = ?1 AND source_id = ?2",
                params![source_kind, source_id],
            )
            .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            for edge in edges {
                if edge.source_kind != source_kind || edge.source_id != source_id {
                    continue;
                }
                tx.execute(
                    "INSERT OR IGNORE INTO page_ref
                       (source_kind, source_id, target_kind, target_id, ref_type, source_extra)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        edge.source_kind,
                        edge.source_id,
                        edge.target_kind,
                        edge.target_id,
                        edge.ref_type,
                        edge.source_extra,
                    ],
                )
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            }
            tx.commit()
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))
        })
        .await
        .unwrap()
    }

    /// Like [`replace_source`] but restricted to a named slice
    /// identified by `ref_types`: only rows whose `ref_type` matches
    /// one of the supplied values are deleted, then `edges` is
    /// inserted. Used when a single source has multiple writers
    /// contributing different `ref_type`s — e.g. `work-item:wi-1`
    /// gets body-mention edges from the work-item upsert, link
    /// edges from the link store, and touched-file edges from the
    /// effort store. Each writer passes only its own ref_types so
    /// the others' rows survive.
    pub async fn replace_source_for_ref_types(
        &self,
        source_kind: &str,
        source_id: &str,
        ref_types: Vec<String>,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        if ref_types.is_empty() {
            return Ok(());
        }
        let db = self.db.clone();
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = db
                .conn()
                .map_err(|e| DomainError::Invalid(format!("pool: {e}")))?;
            let tx = conn
                .transaction()
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            let placeholders: Vec<String> =
                (3..3 + ref_types.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "DELETE FROM page_ref
                 WHERE source_kind = ?1 AND source_id = ?2
                   AND ref_type IN ({})",
                placeholders.join(",")
            );
            let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(2 + ref_types.len());
            params_vec.push(&source_kind);
            params_vec.push(&source_id);
            for rt in &ref_types {
                params_vec.push(rt);
            }
            tx.execute(&sql, &params_vec[..])
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            for edge in edges {
                if edge.source_kind != source_kind || edge.source_id != source_id {
                    continue;
                }
                if !ref_types.contains(&edge.ref_type) {
                    continue;
                }
                tx.execute(
                    "INSERT OR IGNORE INTO page_ref
                       (source_kind, source_id, target_kind, target_id, ref_type, source_extra)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        edge.source_kind,
                        edge.source_id,
                        edge.target_kind,
                        edge.target_id,
                        edge.ref_type,
                        edge.source_extra,
                    ],
                )
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            }
            tx.commit()
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))
        })
        .await
        .unwrap()
    }

    /// Edges that point AT `(target_kind, target_id)` — i.e. the
    /// classic backlinks list. Order is by source-kind then
    /// source-id for stable rendering; callers can re-sort.
    pub async fn list_backlinks(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        let db = self.db.clone();
        let target_kind = target_kind.to_string();
        let target_id = target_id.to_string();
        let limit = limit.unwrap_or(500);
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT source_kind, source_id, target_kind, target_id, ref_type, source_extra
                     FROM page_ref
                     WHERE target_kind = ?1 AND target_id = ?2
                     ORDER BY source_kind, source_id, ref_type
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![target_kind, target_id, limit], row_to_edge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    /// Edges emitted FROM `(source_kind, source_id)` — the page's
    /// own outbound list.
    pub async fn list_outbound(
        &self,
        source_kind: &str,
        source_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        let db = self.db.clone();
        let source_kind = source_kind.to_string();
        let source_id = source_id.to_string();
        let limit = limit.unwrap_or(500);
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT source_kind, source_id, target_kind, target_id, ref_type, source_extra
                     FROM page_ref
                     WHERE source_kind = ?1 AND source_id = ?2
                     ORDER BY target_kind, target_id, ref_type
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![source_kind, source_id, limit], row_to_edge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageRefEdge> {
    Ok(PageRefEdge {
        source_kind: row.get(0)?,
        source_id: row.get(1)?,
        target_kind: row.get(2)?,
        target_id: row.get(3)?,
        ref_type: row.get(4)?,
        source_extra: row.get(5)?,
    })
}

#[async_trait]
pub trait PageRefStore: Send + Sync {
    async fn replace_source(
        &self,
        source_kind: &str,
        source_id: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError>;
    async fn list_backlinks(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError>;
    async fn list_outbound(
        &self,
        source_kind: &str,
        source_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError>;
}

#[async_trait]
impl PageRefStore for SqlitePageRefStore {
    async fn replace_source(
        &self,
        source_kind: &str,
        source_id: &str,
        edges: Vec<PageRefEdge>,
    ) -> Result<(), DomainError> {
        SqlitePageRefStore::replace_source(self, source_kind, source_id, edges).await
    }
    async fn list_backlinks(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        SqlitePageRefStore::list_backlinks(self, target_kind, target_id, limit).await
    }
    async fn list_outbound(
        &self,
        source_kind: &str,
        source_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<PageRefEdge>, DomainError> {
        SqlitePageRefStore::list_outbound(self, source_kind, source_id, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(s_kind: &str, s_id: &str, t_kind: &str, t_id: &str, ref_type: &str) -> PageRefEdge {
        PageRefEdge::new(s_kind, s_id, t_kind, t_id, ref_type)
    }

    #[tokio::test]
    async fn round_trip_single_edge() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge(
            "wiki",
            "architecture",
            "file",
            "src/app.rs",
            "wiki_file_ref",
        );
        store
            .replace_source("wiki", "architecture", vec![e.clone()])
            .await
            .unwrap();

        let inbound = store
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert_eq!(inbound, vec![e.clone()]);

        let outbound = store
            .list_outbound("wiki", "architecture", None)
            .await
            .unwrap();
        assert_eq!(outbound, vec![e]);
    }

    #[tokio::test]
    async fn replace_source_is_idempotent() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let edges = vec![
            edge("wiki", "a", "file", "x.rs", "wiki_file_ref"),
            edge("wiki", "a", "work-item", "wi-1", "wi_body_mention"),
        ];
        store
            .replace_source("wiki", "a", edges.clone())
            .await
            .unwrap();
        store
            .replace_source("wiki", "a", edges.clone())
            .await
            .unwrap();
        let out = store.list_outbound("wiki", "a", None).await.unwrap();
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn replace_source_drops_removed_edges() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        store
            .replace_source(
                "wiki",
                "a",
                vec![
                    edge("wiki", "a", "file", "x.rs", "wiki_file_ref"),
                    edge("wiki", "a", "file", "y.rs", "wiki_file_ref"),
                ],
            )
            .await
            .unwrap();
        // Re-save with only y.rs — x.rs must vanish.
        store
            .replace_source(
                "wiki",
                "a",
                vec![edge("wiki", "a", "file", "y.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        let inbound_x = store.list_backlinks("file", "x.rs", None).await.unwrap();
        let inbound_y = store.list_backlinks("file", "y.rs", None).await.unwrap();
        assert!(inbound_x.is_empty());
        assert_eq!(inbound_y.len(), 1);
    }

    #[tokio::test]
    async fn replace_source_doesnt_touch_other_sources() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        store
            .replace_source(
                "wiki",
                "a",
                vec![edge("wiki", "a", "file", "x.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        store
            .replace_source(
                "wiki",
                "b",
                vec![edge("wiki", "b", "file", "x.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        // Now replace `a` with empty — only `a`'s edges go.
        store.replace_source("wiki", "a", vec![]).await.unwrap();
        let inbound = store.list_backlinks("file", "x.rs", None).await.unwrap();
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound[0].source_id, "b");
    }

    #[tokio::test]
    async fn extra_payload_round_trips() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        let e = edge("wiki", "a", "file", "x.rs", "wiki_file_ref")
            .with_extra(r#"{"line":42,"version":"@HEAD"}"#);
        store
            .replace_source("wiki", "a", vec![e.clone()])
            .await
            .unwrap();
        let got = store.list_outbound("wiki", "a", None).await.unwrap();
        assert_eq!(got, vec![e]);
    }

    #[tokio::test]
    async fn rows_with_mismatched_source_are_skipped() {
        let store = SqlitePageRefStore::new(Database::in_memory());
        // Caller passes an edge with source ("wiki","b") under
        // a replace_source for ("wiki","a"). The mismatched edge
        // must be dropped, not silently inserted, so writers can't
        // accidentally pollute another owner's slice.
        store
            .replace_source(
                "wiki",
                "a",
                vec![edge("wiki", "b", "file", "x.rs", "wiki_file_ref")],
            )
            .await
            .unwrap();
        assert!(store
            .list_outbound("wiki", "b", None)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn target_index_used_for_backlinks_query() {
        // Functional check: thousands of unrelated edges, one
        // matching target, lookup by (target_kind, target_id) is
        // small. (We can't verify the index is HIT without
        // EXPLAIN QUERY PLAN, but a correctness test is fine.)
        let store = SqlitePageRefStore::new(Database::in_memory());
        let mut bulk = Vec::new();
        for i in 0..200 {
            bulk.push(edge(
                "wiki",
                "noise",
                "file",
                &format!("noise/{i}.rs"),
                "wiki_file_ref",
            ));
        }
        bulk.push(edge("wiki", "noise", "file", "target.rs", "wiki_file_ref"));
        store.replace_source("wiki", "noise", bulk).await.unwrap();
        let hits = store
            .list_backlinks("file", "target.rs", None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target_id, "target.rs");
    }
}

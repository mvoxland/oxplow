//! Analytics-flavored stores: page_visit, usage_event, code_quality_*,
//! file_snapshot. They all share the same shape — append-mostly, with
//! recent-window queries — so they live together rather than each
//! getting its own file.

use async_trait::async_trait;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, StreamId, Timestamp};

use crate::database::Database;
use crate::page_ref_projections::finding_edges;
use crate::page_ref_store::SqlitePageRefStore;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .unwrap()
        .trim_matches('"')
        .to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

// ---------------- Page visits ----------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PageVisit {
    pub id: String,
    pub page_kind: String,
    pub page_id: String,
    /// Human-readable label captured at activation time — the same
    /// string the tab strip displays. NULL for legacy rows recorded
    /// before V10 (renderer falls back to page_id for those).
    pub label: Option<String>,
    pub visited_at: Timestamp,
    pub duration_ms: Option<i64>,
    pub thread_id: Option<String>,
}

#[derive(Clone)]
pub struct SqlitePageVisitStore {
    db: Database,
}

impl SqlitePageVisitStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
pub trait PageVisitStore: Send + Sync {
    async fn record(
        &self,
        page_kind: &str,
        page_id: &str,
        label: Option<&str>,
        duration_ms: Option<i64>,
        thread_id: Option<&str>,
    ) -> Result<PageVisit, DomainError>;
    /// Recent visits, optionally scoped to one thread. `None` returns
    /// every visit across threads (the legacy global view).
    async fn list_recent(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<PageVisit>, DomainError>;
    /// Top visited (kind, id) tuples by visit count, optionally scoped to
    /// one thread.
    async fn list_top(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<(String, String, i64)>, DomainError>;
    async fn forget_page(&self, page_kind: &str, page_id: &str) -> Result<(), DomainError>;
    async fn count_by_day(&self, days: u32) -> Result<Vec<(String, i64)>, DomainError>;
    /// Distinct (page_kind, page_id) tuples ordered by most recent visit
    /// — drives the "frequent" rail.
    async fn list_frequent(&self, limit: usize) -> Result<Vec<PageVisit>, DomainError>;
}

#[async_trait]
impl PageVisitStore for SqlitePageVisitStore {
    async fn record(
        &self,
        page_kind: &str,
        page_id: &str,
        label: Option<&str>,
        duration_ms: Option<i64>,
        thread_id: Option<&str>,
    ) -> Result<PageVisit, DomainError> {
        let db = self.db.clone();
        let page_kind = page_kind.to_string();
        let page_id = page_id.to_string();
        let label = label.map(|s| s.to_string());
        let thread_id = thread_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let id = format!("pv-{}", uuid::Uuid::new_v4().simple());
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO page_visit (id, page_kind, page_id, label, visited_at, duration_ms, thread_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, page_kind, page_id, label, ts_to_string(now), duration_ms, thread_id],
                )?;
                Ok(())
            })?;
            Ok(PageVisit {
                id,
                page_kind,
                page_id,
                label,
                visited_at: now,
                duration_ms,
                thread_id,
            })
        })
        .await
        .unwrap()
    }

    async fn list_recent(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<PageVisit>, DomainError> {
        let db = self.db.clone();
        let thread_id = thread_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = if thread_id.is_some() {
                    conn.prepare(
                        "SELECT id, page_kind, page_id, label, visited_at, duration_ms, thread_id
                         FROM page_visit
                         WHERE thread_id = ?2
                         ORDER BY visited_at DESC LIMIT ?1",
                    )?
                } else {
                    conn.prepare(
                        "SELECT id, page_kind, page_id, label, visited_at, duration_ms, thread_id
                         FROM page_visit
                         ORDER BY visited_at DESC LIMIT ?1",
                    )?
                };
                let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<PageVisit> {
                    let id: String = row.get(0)?;
                    let page_kind: String = row.get(1)?;
                    let page_id: String = row.get(2)?;
                    let label: Option<String> = row.get(3)?;
                    let visited_at: String = row.get(4)?;
                    let duration_ms: Option<i64> = row.get(5)?;
                    let thread_id: Option<String> = row.get(6)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(PageVisit {
                        id,
                        page_kind,
                        page_id,
                        label,
                        visited_at: string_to_ts(&visited_at).map_err(map_err)?,
                        duration_ms,
                        thread_id,
                    })
                };
                if let Some(tid) = thread_id.as_deref() {
                    let rows: rusqlite::Result<Vec<_>> = stmt
                        .query_map(params![limit as i64, tid], map_row)?
                        .collect();
                    rows
                } else {
                    let rows: rusqlite::Result<Vec<_>> =
                        stmt.query_map(params![limit as i64], map_row)?.collect();
                    rows
                }
            })
        })
        .await
        .unwrap()
    }

    async fn list_top(
        &self,
        limit: usize,
        thread_id: Option<&str>,
    ) -> Result<Vec<(String, String, i64)>, DomainError> {
        let db = self.db.clone();
        let thread_id = thread_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let map_row = |row: &rusqlite::Row<'_>| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                };
                if let Some(tid) = thread_id.as_deref() {
                    let mut stmt = conn.prepare(
                        "SELECT page_kind, page_id, COUNT(*) AS visits
                         FROM page_visit
                         WHERE thread_id = ?2
                         GROUP BY page_kind, page_id
                         ORDER BY visits DESC
                         LIMIT ?1",
                    )?;
                    let rows: rusqlite::Result<Vec<_>> = stmt
                        .query_map(params![limit as i64, tid], map_row)?
                        .collect();
                    rows
                } else {
                    let mut stmt = conn.prepare(
                        "SELECT page_kind, page_id, COUNT(*) AS visits
                         FROM page_visit
                         GROUP BY page_kind, page_id
                         ORDER BY visits DESC
                         LIMIT ?1",
                    )?;
                    let rows: rusqlite::Result<Vec<_>> =
                        stmt.query_map(params![limit as i64], map_row)?.collect();
                    rows
                }
            })
        })
        .await
        .unwrap()
    }

    async fn forget_page(&self, page_kind: &str, page_id: &str) -> Result<(), DomainError> {
        let db = self.db.clone();
        let page_kind = page_kind.to_string();
        let page_id = page_id.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "DELETE FROM page_visit WHERE page_kind = ?1 AND page_id = ?2",
                    params![page_kind, page_id],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn list_frequent(&self, limit: usize) -> Result<Vec<PageVisit>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                // Most-recent visit per page, ordered by visit count desc.
                let mut stmt = conn.prepare(
                    "SELECT id, page_kind, page_id, label, visited_at, duration_ms, thread_id
                     FROM page_visit pv
                     WHERE id = (
                         SELECT id FROM page_visit pv2
                         WHERE pv2.page_kind = pv.page_kind AND pv2.page_id = pv.page_id
                         ORDER BY visited_at DESC LIMIT 1
                     )
                     ORDER BY (
                         SELECT COUNT(*) FROM page_visit pv3
                         WHERE pv3.page_kind = pv.page_kind AND pv3.page_id = pv.page_id
                     ) DESC
                     LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], |row| {
                    let id: String = row.get(0)?;
                    let page_kind: String = row.get(1)?;
                    let page_id: String = row.get(2)?;
                    let label: Option<String> = row.get(3)?;
                    let visited_at: String = row.get(4)?;
                    let duration_ms: Option<i64> = row.get(5)?;
                    let thread_id: Option<String> = row.get(6)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(PageVisit {
                        id,
                        page_kind,
                        page_id,
                        label,
                        visited_at: string_to_ts(&visited_at).map_err(map_err)?,
                        duration_ms,
                        thread_id,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn count_by_day(&self, days: u32) -> Result<Vec<(String, i64)>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT substr(visited_at, 1, 10) AS day, COUNT(*) AS visits
                     FROM page_visit
                     WHERE visited_at >= datetime('now', ?1)
                     GROUP BY day
                     ORDER BY day DESC",
                )?;
                let cutoff = format!("-{} days", days);
                let rows = stmt.query_map(params![cutoff], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }
}

// ---------------- Usage events ----------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct UsageEvent {
    pub id: String,
    pub kind: String,
    pub payload_json: String,
    pub occurred_at: Timestamp,
}

/// Per-key aggregation of usage events. Returned by
/// `SqliteUsageStore::list_recent_rollup` for callers that want
/// "most-recently-touched X" lists rather than the raw event log
/// (e.g. the WikiActivityBar's recent-files strip).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct UsageRollup {
    pub kind: String,
    pub key: String,
    pub last_at: Timestamp,
    pub count: u32,
}

#[derive(Clone)]
pub struct SqliteUsageStore {
    db: Database,
}

impl SqliteUsageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record(
        &self,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<UsageEvent, DomainError> {
        let db = self.db.clone();
        let kind = kind.to_string();
        let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        tokio::task::spawn_blocking(move || {
            let id = format!("ue-{}", uuid::Uuid::new_v4().simple());
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO usage_event (id, kind, payload_json, occurred_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, kind, payload_json, ts_to_string(now)],
                )?;
                Ok(())
            })?;
            Ok(UsageEvent {
                id,
                kind,
                payload_json,
                occurred_at: now,
            })
        })
        .await
        .unwrap()
    }

    /// Group recent events of a single `kind` by the per-row key
    /// extracted from `payload_json`. The extraction tries the same
    /// candidate fields that `commands::usage::extract_key` does
    /// (`key` → `slug` → `path` → `id` → `itemId` / `item_id` →
    /// `noteId` / `note_id`); rows whose payload yields no key are
    /// dropped. When `stream_id` is `Some`, only events whose payload
    /// includes that `streamId` (or `stream_id`) are counted.
    pub async fn list_recent_rollup(
        &self,
        kind: &str,
        stream_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<UsageRollup>, DomainError> {
        let db = self.db.clone();
        let kind = kind.to_string();
        let stream_id = stream_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                // COALESCE over the canonical key fields. Mirrors
                // `commands::usage::extract_key` so a renderer
                // listening to UsageRecorded events sees keys agreeing
                // with what shows up in the rollup.
                let stream_filter = if stream_id.is_some() {
                    "AND COALESCE(json_extract(payload_json, '$.streamId'), \
                                  json_extract(payload_json, '$.stream_id')) = ?3"
                } else {
                    ""
                };
                let sql = format!(
                    "SELECT \
                       COALESCE( \
                         json_extract(payload_json, '$.key'), \
                         json_extract(payload_json, '$.slug'), \
                         json_extract(payload_json, '$.path'), \
                         json_extract(payload_json, '$.id'), \
                         json_extract(payload_json, '$.itemId'), \
                         json_extract(payload_json, '$.item_id'), \
                         json_extract(payload_json, '$.noteId'), \
                         json_extract(payload_json, '$.note_id') \
                       ) AS key, \
                       MAX(occurred_at) AS last_at, \
                       COUNT(*) AS cnt \
                     FROM usage_event \
                     WHERE kind = ?1 {stream_filter} \
                     GROUP BY key \
                     HAVING key IS NOT NULL AND key != '' \
                     ORDER BY last_at DESC \
                     LIMIT ?2"
                );
                let mut stmt = conn.prepare(&sql)?;
                let map_err = |e: DomainError| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                };
                let collect = |row: &rusqlite::Row| -> rusqlite::Result<UsageRollup> {
                    let key: String = row.get(0)?;
                    let last_at: String = row.get(1)?;
                    let cnt: i64 = row.get(2)?;
                    Ok(UsageRollup {
                        kind: kind.clone(),
                        key,
                        last_at: string_to_ts(&last_at).map_err(map_err)?,
                        count: cnt.max(0) as u32,
                    })
                };
                let rows: Vec<UsageRollup> = if let Some(sid) = stream_id.as_deref() {
                    stmt.query_map(params![kind, limit as i64, sid], collect)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(params![kind, limit as i64], collect)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
        })
        .await
        .unwrap()
    }

    pub async fn list_recent(&self, limit: usize) -> Result<Vec<UsageEvent>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, payload_json, occurred_at FROM usage_event
                     ORDER BY occurred_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], |row| {
                    let id: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let payload_json: String = row.get(2)?;
                    let occurred_at: String = row.get(3)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(UsageEvent {
                        id,
                        kind,
                        payload_json,
                        occurred_at: string_to_ts(&occurred_at).map_err(map_err)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }
}

// ---------------- Code quality ----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CodeQualityScanStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CodeQualityScan {
    pub id: i64,
    pub tool: String,
    pub scope: String,
    pub status: CodeQualityScanStatus,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub error: Option<String>,
    /// Tree version the scan ran against. `"disk" | "ref" | "snapshot"`.
    /// Backfilled to `"disk"` for pre-V9 rows.
    pub tree_version_kind: String,
    /// Identifier for the version: ref-spec or snapshot id; null for
    /// disk.
    pub tree_version_value: Option<String>,
    /// File filter applied: `"all"` or `"explicit:<sha-of-paths>"`.
    /// Backfilled to `"all"` for pre-V9 rows.
    pub file_filter: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CodeQualityFinding {
    pub id: i64,
    pub scan_id: i64,
    pub path: String,
    pub start_line: i32,
    pub end_line: i32,
    pub kind: String,
    pub metric_value: f64,
    pub extra_json: Option<String>,
}

fn row_to_scan(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodeQualityScan> {
    let id: i64 = row.get(0)?;
    let tool: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let status: String = row.get(3)?;
    let started_at: String = row.get(4)?;
    let ended_at: Option<String> = row.get(5)?;
    let error: Option<String> = row.get(6)?;
    let tree_version_kind: Option<String> = row.get(7)?;
    let tree_version_value: Option<String> = row.get(8)?;
    let file_filter: Option<String> = row.get(9)?;
    let status = match status.as_str() {
        "pending" => CodeQualityScanStatus::Pending,
        "running" => CodeQualityScanStatus::Running,
        "done" => CodeQualityScanStatus::Done,
        _ => CodeQualityScanStatus::Failed,
    };
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(CodeQualityScan {
        id,
        tool,
        scope,
        status,
        started_at: string_to_ts(&started_at).map_err(map_err)?,
        ended_at: ended_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        error,
        // Default backfill matches the V9 migration's UPDATE.
        tree_version_kind: tree_version_kind.unwrap_or_else(|| "disk".into()),
        tree_version_value,
        file_filter: file_filter.unwrap_or_else(|| "all".into()),
    })
}

#[derive(Clone)]
pub struct SqliteCodeQualityStore {
    db: Database,
    page_refs: Option<SqlitePageRefStore>,
}

impl SqliteCodeQualityStore {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            page_refs: None,
        }
    }

    pub fn with_page_refs(mut self, store: SqlitePageRefStore) -> Self {
        self.page_refs = Some(store);
        self
    }

    pub async fn create_scan(&self, tool: &str, scope: &str) -> Result<i64, DomainError> {
        // Legacy entry point: callers that haven't been ported to the
        // versioned API land here. Default to ("disk", null, "all"),
        // matching the implicit pre-V9 behavior.
        self.create_scan_with(tool, scope, "disk", None, "all")
            .await
    }

    /// Versioned create_scan. Tags the row with the tree version it
    /// ran against (`disk` / `ref:<spec>` / `snapshot:<id>`) and the
    /// file filter applied (`all` / `explicit:<sha>`), so consumers
    /// can ask for "the latest scan at this commit" without confusing
    /// results from a different version.
    pub async fn create_scan_with(
        &self,
        tool: &str,
        scope: &str,
        tree_version_kind: &str,
        tree_version_value: Option<&str>,
        file_filter: &str,
    ) -> Result<i64, DomainError> {
        let db = self.db.clone();
        let tool = tool.to_string();
        let scope = scope.to_string();
        let kind = tree_version_kind.to_string();
        let value = tree_version_value.map(str::to_string);
        let filter = file_filter.to_string();
        tokio::task::spawn_blocking(move || {
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO code_quality_scan
                       (tool, scope, status, started_at,
                        tree_version_kind, tree_version_value, file_filter)
                     VALUES (?1, ?2, 'pending', ?3, ?4, ?5, ?6)",
                    params![tool, scope, ts_to_string(now), kind, value, filter,],
                )?;
                Ok(conn.last_insert_rowid())
            })
        })
        .await
        .unwrap()
    }

    pub async fn finish_scan(
        &self,
        id: i64,
        status: CodeQualityScanStatus,
        error: Option<String>,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let status_str = match status {
            CodeQualityScanStatus::Pending => "pending",
            CodeQualityScanStatus::Running => "running",
            CodeQualityScanStatus::Done => "done",
            CodeQualityScanStatus::Failed => "failed",
        };
        tokio::task::spawn_blocking(move || {
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE code_quality_scan SET status = ?2, ended_at = ?3, error = ?4 WHERE id = ?1",
                    params![id, status_str, ts_to_string(now), error],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    pub async fn append_finding(
        &self,
        scan_id: i64,
        finding: CodeQualityFinding,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let finding_clone = finding.clone();
        let finding_id: i64 = tokio::task::spawn_blocking(move || -> Result<i64, DomainError> {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO code_quality_finding
                       (scan_id, path, start_line, end_line, kind, metric_value, extra_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        scan_id,
                        finding_clone.path,
                        finding_clone.start_line,
                        finding_clone.end_line,
                        finding_clone.kind,
                        finding_clone.metric_value,
                        finding_clone.extra_json,
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })
        })
        .await
        .unwrap()?;
        if let Some(refs) = &self.page_refs {
            let edges = finding_edges(&finding_id.to_string(), &finding.path);
            refs.replace_source("finding", &finding_id.to_string(), edges)
                .await?;
        }
        Ok(())
    }

    /// All findings across every scan as `(rowid, path)`. Used by
    /// the page-ref backfill — each finding row owns one
    /// `(finding:<rowid>) -> (file:<path>)` edge.
    pub async fn list_all_findings_for_backfill(&self) -> Result<Vec<(i64, String)>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT id, path FROM code_quality_finding")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    pub async fn list_scans(&self, limit: usize) -> Result<Vec<CodeQualityScan>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, tool, scope, status, started_at, ended_at, error,
                            tree_version_kind, tree_version_value, file_filter
                     FROM code_quality_scan ORDER BY started_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], row_to_scan)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    /// Find the most recent `done` scan for `(tool, treeVersion,
    /// fileFilter)`. Returns `None` if there's no matching scan, so
    /// the caller can render a "Scan now" CTA instead of an empty
    /// findings list.
    pub async fn find_latest_done_scan(
        &self,
        tool: &str,
        tree_version_kind: &str,
        tree_version_value: Option<&str>,
        file_filter: &str,
    ) -> Result<Option<CodeQualityScan>, DomainError> {
        let db = self.db.clone();
        let tool = tool.to_string();
        let kind = tree_version_kind.to_string();
        let value = tree_version_value.map(str::to_string);
        let filter = file_filter.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, tool, scope, status, started_at, ended_at, error,
                            tree_version_kind, tree_version_value, file_filter
                     FROM code_quality_scan
                     WHERE tool = ?1
                       AND status = 'done'
                       AND tree_version_kind = ?2
                       AND ((?3 IS NULL AND tree_version_value IS NULL)
                            OR tree_version_value = ?3)
                       AND file_filter = ?4
                     ORDER BY started_at DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![tool, kind, value, filter], row_to_scan)?;
                match rows.next() {
                    Some(Ok(scan)) => Ok(Some(scan)),
                    Some(Err(e)) => Err(e),
                    None => Ok(None),
                }
            })
        })
        .await
        .unwrap()
    }

    pub async fn list_findings(
        &self,
        scan_id: i64,
    ) -> Result<Vec<CodeQualityFinding>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, scan_id, path, start_line, end_line, kind, metric_value, extra_json
                     FROM code_quality_finding WHERE scan_id = ?1 ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(params![scan_id], |row| {
                    Ok(CodeQualityFinding {
                        id: row.get(0)?,
                        scan_id: row.get(1)?,
                        path: row.get(2)?,
                        start_line: row.get(3)?,
                        end_line: row.get(4)?,
                        kind: row.get(5)?,
                        metric_value: row.get(6)?,
                        extra_json: row.get(7)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }
}

// ---------------- File snapshots ----------------

fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileSnapshot> {
    let id: i64 = row.get(0)?;
    let stream_id: String = row.get(1)?;
    let path: String = row.get(2)?;
    let blob_hash: Option<String> = row.get(3)?;
    let size_bytes: i64 = row.get(4)?;
    let captured_at: String = row.get(5)?;
    let oversize: i32 = row.get(6)?;
    // snapshot_id / mtime_ms only present when the SELECT asks for
    // them (V13 / V15+); older 7-column callers see them as missing
    // and we treat that as None.
    let snapshot_id: Option<i64> = row.get(7).ok().flatten();
    let mtime_ms: Option<i64> = row.get(8).ok().flatten();
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(FileSnapshot {
        id,
        stream_id: StreamId::from(stream_id),
        path,
        blob_hash,
        size_bytes,
        captured_at: string_to_ts(&captured_at).map_err(map_err)?,
        oversize: oversize != 0,
        snapshot_id,
        mtime_ms,
    })
}

/// Aggregate created/modified/deleted counts for the file rows
/// captured under one snapshot. Derived by comparing each child
/// row's `blob_hash` to the most-recent prior row for the same
/// `(stream_id, path)`. Powers the Local History dashboard's
/// per-snapshot stats column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Default)]
pub struct SnapshotStats {
    pub created: i64,
    pub modified: i64,
    pub deleted: i64,
    pub total: i64,
}

/// One row per file captured under a snapshot, in the shape the
/// renderer's change-analysis pipeline expects. `status` mirrors
/// `BranchChangeEntry`'s set (`added`/`modified`/`deleted`) so the
/// shared SummaryCard / ChangeAnalysisPanel can render snapshot
/// changes alongside git ones. `current_file_id` is the row in
/// `file_snapshot` captured for this snapshot; `prior_file_id` is
/// the most recent prior capture of the same `(stream_id, path)`,
/// used to pull the "before" blob bytes for diff + function
/// analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SnapshotChangeEntry {
    pub path: String,
    pub status: String,
    pub current_file_id: i64,
    pub prior_file_id: Option<i64>,
    pub oversize: bool,
}

/// `snapshot` row — one per `request_snapshot()` call that had
/// dirty files. Groups the `file_snapshot` rows captured in that
/// batch. See [[crates/oxplow-db/migrations/V13__snapshot.sql]].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Snapshot {
    pub id: i64,
    pub stream_id: StreamId,
    pub created_at: Timestamp,
    pub file_count: i64,
    /// 40-char git sha corresponding to this snapshot's worktree
    /// state. Populated only when the worktree was clean at capture
    /// time (no tracked-file changes, no non-ignored untracked
    /// files); `None` when the tree was dirty or the directory isn't
    /// a git repo at all. Not unique — multiple snapshots can share
    /// the same commit when local history captures files git doesn't
    /// track.
    pub git_commit: Option<String>,
}

/// Most-recent stat (hash + size + mtime) for a single path. The
/// startup sweep uses this to short-circuit the read+hash pass when
/// `(size_bytes, mtime_ms)` match the file on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestStat {
    pub blob_hash: Option<String>,
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FileSnapshot {
    pub id: i64,
    pub stream_id: StreamId,
    pub path: String,
    pub blob_hash: Option<String>,
    pub size_bytes: i64,
    pub captured_at: Timestamp,
    pub oversize: bool,
    /// `snapshot.id` this row was captured under, or `None` for
    /// pre-V13 rows that predate the snapshot grouping table.
    pub snapshot_id: Option<i64>,
    /// File mtime in unix milliseconds at capture time. NULL for
    /// rows written before V15 added the column. The startup sweep
    /// uses `(size_bytes, mtime_ms)` as a fast equality check: if
    /// both match the current stat, the file is presumed unchanged
    /// and the bytes aren't re-read or re-hashed.
    pub mtime_ms: Option<i64>,
}

#[derive(Clone)]
pub struct SqliteSnapshotStore {
    db: Database,
}

impl SqliteSnapshotStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn capture(&self, snap: FileSnapshot) -> Result<i64, DomainError> {
        Ok(self.capture_batch(vec![snap]).await?[0])
    }

    /// Insert N `file_snapshot` rows in a single transaction. Returns
    /// the new row ids in input order. Used by
    /// `SnapshotCaptureService::request_snapshot` to flush the entire
    /// drained dirty set with one DB round-trip — at 34k rows the
    /// per-INSERT autocommit overhead of `capture()` dominates wall
    /// time, and the transaction shape collapses it to a single fsync.
    pub async fn capture_batch(&self, snaps: Vec<FileSnapshot>) -> Result<Vec<i64>, DomainError> {
        if snaps.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = db
                .conn()
                .map_err(|e| DomainError::Invalid(format!("pool: {e}")))?;
            let tx = conn
                .transaction()
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            let mut ids = Vec::with_capacity(snaps.len());
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO file_snapshot
                           (stream_id, path, blob_hash, size_bytes, captured_at, oversize,
                            snapshot_id, mtime_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    )
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                for snap in &snaps {
                    stmt.execute(params![
                        snap.stream_id.as_str(),
                        snap.path,
                        snap.blob_hash,
                        snap.size_bytes,
                        ts_to_string(snap.captured_at),
                        if snap.oversize { 1 } else { 0 },
                        snap.snapshot_id,
                        snap.mtime_ms,
                    ])
                    .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
                    ids.push(tx.last_insert_rowid());
                }
            }
            tx.commit()
                .map_err(|e| DomainError::Invalid(format!("sql: {e}")))?;
            Ok(ids)
        })
        .await
        .unwrap()
    }

    /// Insert a new `snapshot` row and return its id. Callers (e.g.
    /// `SnapshotCaptureService::request_snapshot`) only do this when
    /// they have dirty files to capture — empty requests reuse
    /// `latest_snapshot_id_for_stream`.
    pub async fn create_snapshot(&self, stream_id: StreamId) -> Result<i64, DomainError> {
        let db = self.db.clone();
        let now = ts_to_string(Timestamp::now());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO snapshot (stream_id, created_at) VALUES (?1, ?2)",
                    params![stream_id.as_str(), now],
                )?;
                Ok(conn.last_insert_rowid())
            })
        })
        .await
        .unwrap()
    }

    /// Read the `git_commit` column for a snapshot, if recorded.
    pub async fn get_snapshot_git_commit(
        &self,
        snapshot_id: i64,
    ) -> Result<Option<String>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.query_row(
                    "SELECT git_commit FROM snapshot WHERE id = ?1",
                    params![snapshot_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map(|opt| opt.flatten())
            })
        })
        .await
        .unwrap()
    }

    /// Pin a snapshot to a git commit sha. Called by the capture
    /// layer immediately after `create_snapshot` when the worktree
    /// was clean.
    pub async fn set_snapshot_git_commit(
        &self,
        snapshot_id: i64,
        sha: String,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE snapshot SET git_commit = ?1 WHERE id = ?2",
                    params![sha, snapshot_id],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    /// Most recent `snapshot.id` for the stream. Returns `None` when
    /// no snapshots exist yet for the stream.
    pub async fn latest_snapshot_id_for_stream(
        &self,
        stream_id: StreamId,
    ) -> Result<Option<i64>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let row: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM snapshot WHERE stream_id = ?1
                         ORDER BY created_at DESC, id DESC LIMIT 1",
                        params![stream_id.as_str()],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(row)
            })
        })
        .await
        .unwrap()
    }

    /// Snapshot rows for a stream, newest first.
    pub async fn list_snapshots_for_stream(
        &self,
        stream_id: &str,
        limit: usize,
    ) -> Result<Vec<Snapshot>, DomainError> {
        let db = self.db.clone();
        let stream_id = stream_id.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT s.id, s.stream_id, s.created_at,
                            (SELECT COUNT(*) FROM file_snapshot f
                             WHERE f.snapshot_id = s.id) AS file_count,
                            s.git_commit
                     FROM snapshot s
                     WHERE s.stream_id = ?1
                     ORDER BY s.created_at DESC, s.id DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![stream_id, limit as i64], |row| {
                    let id: i64 = row.get(0)?;
                    let stream_id: String = row.get(1)?;
                    let created_at: String = row.get(2)?;
                    let file_count: i64 = row.get(3)?;
                    let git_commit: Option<String> = row.get(4)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(Snapshot {
                        id,
                        stream_id: StreamId::from(stream_id),
                        created_at: string_to_ts(&created_at).map_err(map_err)?,
                        file_count,
                        git_commit,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    /// Aggregate counts of created/modified/deleted files in a
    /// snapshot — derived by comparing each child row's `blob_hash`
    /// to the most-recent prior row for the same `(stream_id, path)`.
    /// The `idx_file_snapshot_stream_path` index covers the prior-row
    /// lookup so this stays cheap even with multi-million-row history.
    pub async fn stats_for_snapshot(&self, snapshot_id: i64) -> Result<SnapshotStats, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.query_row(
                    "SELECT
                       COALESCE(SUM(CASE WHEN fs.blob_hash IS NULL THEN 1 ELSE 0 END), 0) AS deleted,
                       COALESCE(SUM(CASE WHEN fs.blob_hash IS NOT NULL AND prev_hash IS NULL THEN 1 ELSE 0 END), 0) AS created,
                       COALESCE(SUM(CASE WHEN fs.blob_hash IS NOT NULL AND prev_hash IS NOT NULL THEN 1 ELSE 0 END), 0) AS modified,
                       COUNT(*) AS total
                     FROM (
                       SELECT
                         f.blob_hash,
                         (SELECT p.blob_hash FROM file_snapshot p
                          WHERE p.stream_id = f.stream_id
                            AND p.path = f.path
                            AND p.id < f.id
                          ORDER BY p.id DESC LIMIT 1) AS prev_hash
                       FROM file_snapshot f
                       WHERE f.snapshot_id = ?1
                     ) fs",
                    params![snapshot_id],
                    |row| {
                        Ok(SnapshotStats {
                            deleted: row.get(0)?,
                            created: row.get(1)?,
                            modified: row.get(2)?,
                            total: row.get(3)?,
                        })
                    },
                )
            })
        })
        .await
        .unwrap()
    }

    /// `SnapshotChangeEntry` rows for one snapshot. Pairs every
    /// child row with its most-recent prior `(stream_id, path)` row
    /// and labels the status (`added`/`modified`/`deleted`) so the
    /// renderer can feed the same shape into the shared change-
    /// analysis pipeline used by Git commits.
    pub async fn list_changes_for_snapshot(
        &self,
        snapshot_id: i64,
    ) -> Result<Vec<SnapshotChangeEntry>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT
                       f.id, f.path, f.blob_hash, f.oversize,
                       (SELECT p.id FROM file_snapshot p
                        WHERE p.stream_id = f.stream_id
                          AND p.path = f.path
                          AND p.id < f.id
                        ORDER BY p.id DESC LIMIT 1) AS prior_id,
                       (SELECT p.blob_hash FROM file_snapshot p
                        WHERE p.stream_id = f.stream_id
                          AND p.path = f.path
                          AND p.id < f.id
                        ORDER BY p.id DESC LIMIT 1) AS prior_hash
                     FROM file_snapshot f
                     WHERE f.snapshot_id = ?1
                     ORDER BY f.path ASC",
                )?;
                let rows = stmt.query_map(params![snapshot_id], |row| {
                    let current_file_id: i64 = row.get(0)?;
                    let path: String = row.get(1)?;
                    let blob_hash: Option<String> = row.get(2)?;
                    let oversize: i32 = row.get(3)?;
                    let prior_file_id: Option<i64> = row.get(4)?;
                    let prior_hash: Option<String> = row.get(5)?;
                    let status = match (&blob_hash, &prior_hash) {
                        (None, _) => "deleted",
                        (Some(_), None) => "added",
                        (Some(_), Some(_)) => "modified",
                    }
                    .to_string();
                    Ok(SnapshotChangeEntry {
                        path,
                        status,
                        current_file_id,
                        prior_file_id,
                        oversize: oversize != 0,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    pub async fn list_files_for_snapshot(
        &self,
        snapshot_id: i64,
    ) -> Result<Vec<FileSnapshot>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, oversize,
                            snapshot_id
                     FROM file_snapshot WHERE snapshot_id = ?1 ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(params![snapshot_id], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    pub async fn get(&self, id: i64) -> Result<Option<FileSnapshot>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, oversize, snapshot_id, mtime_ms
                     FROM file_snapshot WHERE id = ?1",
                )?;
                let mut rows = stmt.query_map(params![id], row_to_snapshot)?;
                rows.next().transpose()
            })
        })
        .await
        .unwrap()
    }

    pub async fn list_for_stream(
        &self,
        stream_id: &str,
        limit: usize,
    ) -> Result<Vec<FileSnapshot>, DomainError> {
        let db = self.db.clone();
        let stream_id = stream_id.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, oversize, snapshot_id, mtime_ms
                     FROM file_snapshot WHERE stream_id = ?1
                     ORDER BY captured_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![stream_id, limit as i64], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    /// Most recent `(blob_hash, size_bytes, mtime_ms)` per path
    /// across the whole table. Used by the startup sweep: when the
    /// current file's `(size, mtime)` matches the stored values, the
    /// bytes are presumed identical and we skip the read + hash.
    /// `mtime_ms` is `None` for pre-V15 rows that predate the column.
    pub async fn latest_stat_per_path(
        &self,
    ) -> Result<std::collections::HashMap<String, LatestStat>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT s.path, s.blob_hash, s.size_bytes, s.mtime_ms
                     FROM file_snapshot s
                     JOIN (
                       SELECT path, MAX(id) AS max_id
                       FROM file_snapshot GROUP BY path
                     ) m ON m.path = s.path AND m.max_id = s.id",
                )?;
                let rows = stmt.query_map([], |row| {
                    let path: String = row.get(0)?;
                    let hash: Option<String> = row.get(1)?;
                    let size: i64 = row.get(2)?;
                    let mtime: Option<i64> = row.get(3)?;
                    Ok((
                        path,
                        LatestStat {
                            blob_hash: hash,
                            size_bytes: size,
                            mtime_ms: mtime,
                        },
                    ))
                })?;
                let mut out = std::collections::HashMap::new();
                for row in rows {
                    let (p, stat) = row?;
                    out.insert(p, stat);
                }
                Ok(out)
            })
        })
        .await
        .unwrap()
    }

    /// Distinct non-null `blob_hash` values referenced by any row.
    /// Used by blob GC to decide which on-disk content is still live.
    pub async fn referenced_blob_hashes(
        &self,
    ) -> Result<std::collections::HashSet<String>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT blob_hash FROM file_snapshot WHERE blob_hash IS NOT NULL",
                )?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()
            })
        })
        .await
        .unwrap()
    }

    /// Test-only: rewrite the oldest row for `path` to a given
    /// `captured_at`. Lets cleanup tests construct rows that fall
    /// outside a retention window without time-traveling the clock.
    #[doc(hidden)]
    pub async fn backdate_for_test(self: std::sync::Arc<Self>, path: &str, ts: Timestamp) {
        let db = self.db.clone();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| -> rusqlite::Result<()> {
                conn.execute(
                    "UPDATE file_snapshot SET captured_at = ?1
                     WHERE id = (SELECT MIN(id) FROM file_snapshot WHERE path = ?2)",
                    params![ts_to_string(ts), path],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
    }

    /// Delete snapshot rows whose `captured_at` is older than
    /// `cutoff`, except the most-recent row per path (so every
    /// file keeps at least one history entry no matter how old).
    /// Returns the number of rows deleted.
    pub async fn prune_older_than(&self, cutoff: Timestamp) -> Result<u64, DomainError> {
        let db = self.db.clone();
        let cutoff_str = ts_to_string(cutoff);
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let n = conn.execute(
                    "DELETE FROM file_snapshot
                     WHERE captured_at < ?1
                       AND id NOT IN (
                         SELECT MAX(id) FROM file_snapshot GROUP BY path
                       )",
                    params![cutoff_str],
                )?;
                Ok(n as u64)
            })
        })
        .await
        .unwrap()
    }

    pub async fn list_for_path(&self, path: &str) -> Result<Vec<FileSnapshot>, DomainError> {
        let db = self.db.clone();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, stream_id, path, blob_hash, size_bytes, captured_at, oversize, snapshot_id, mtime_ms
                     FROM file_snapshot WHERE path = ?1 ORDER BY captured_at DESC",
                )?;
                let rows = stmt.query_map(params![path], row_to_snapshot)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn page_visit_record_then_recent() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store
            .record("wiki", "abc", None, Some(1234), None)
            .await
            .unwrap();
        store
            .record("task", "wi-1", None, None, None)
            .await
            .unwrap();
        let recent = store.list_recent(10, None).await.unwrap();
        assert_eq!(recent.len(), 2);
        // newest first
        assert_eq!(recent[0].page_kind, "task");
    }

    #[tokio::test]
    async fn page_visit_top_groups_correctly() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store.record("wiki", "a", None, None, None).await.unwrap();
        store.record("wiki", "a", None, None, None).await.unwrap();
        store.record("wiki", "b", None, None, None).await.unwrap();
        let top = store.list_top(10, None).await.unwrap();
        assert_eq!(top[0].1, "a");
        assert_eq!(top[0].2, 2);
    }

    #[tokio::test]
    async fn page_visit_forget_clears_only_target() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store.record("wiki", "a", None, None, None).await.unwrap();
        store.record("wiki", "b", None, None, None).await.unwrap();
        store.forget_page("wiki", "a").await.unwrap();
        let recent = store.list_recent(10, None).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].page_id, "b");
    }

    #[tokio::test]
    async fn page_visit_recent_filters_by_thread() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store
            .record("wiki", "a", None, None, Some("b-1"))
            .await
            .unwrap();
        store
            .record("wiki", "b", None, None, Some("b-2"))
            .await
            .unwrap();
        store.record("wiki", "c", None, None, None).await.unwrap();
        let in_thread = store.list_recent(10, Some("b-1")).await.unwrap();
        assert_eq!(in_thread.len(), 1);
        assert_eq!(in_thread[0].page_id, "a");
        let global = store.list_recent(10, None).await.unwrap();
        assert_eq!(global.len(), 3);
    }

    #[tokio::test]
    async fn page_visit_top_filters_by_thread() {
        let store = SqlitePageVisitStore::new(Database::in_memory());
        store
            .record("wiki", "a", None, None, Some("b-1"))
            .await
            .unwrap();
        store
            .record("wiki", "a", None, None, Some("b-1"))
            .await
            .unwrap();
        store
            .record("wiki", "a", None, None, Some("b-2"))
            .await
            .unwrap();
        store
            .record("wiki", "b", None, None, Some("b-1"))
            .await
            .unwrap();
        let in_thread = store.list_top(10, Some("b-1")).await.unwrap();
        // a appears twice in b-1, b once
        assert_eq!(in_thread[0].1, "a");
        assert_eq!(in_thread[0].2, 2);
        assert_eq!(in_thread[1].1, "b");
        assert_eq!(in_thread[1].2, 1);
    }

    #[tokio::test]
    async fn usage_event_round_trip() {
        let store = SqliteUsageStore::new(Database::in_memory());
        store
            .record("agent_turn_started", serde_json::json!({"thread": "b-1"}))
            .await
            .unwrap();
        let recent = store.list_recent(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].kind, "agent_turn_started");
    }

    #[tokio::test]
    async fn usage_rollup_groups_by_key_and_orders_by_recency() {
        let store = SqliteUsageStore::new(Database::in_memory());
        // Two hits on a.ts in stream s-1, one on b.ts (s-1), one on
        // c.ts in a different stream s-2 (must be filtered out).
        for path in ["a.ts", "a.ts", "b.ts"] {
            store
                .record(
                    "editor-file",
                    serde_json::json!({"path": path, "streamId": "s-1"}),
                )
                .await
                .unwrap();
        }
        store
            .record(
                "editor-file",
                serde_json::json!({"path": "c.ts", "streamId": "s-2"}),
            )
            .await
            .unwrap();
        // A different kind in the same stream — must not appear.
        store
            .record("wiki", serde_json::json!({"slug": "z", "streamId": "s-1"}))
            .await
            .unwrap();

        let rollup = store
            .list_recent_rollup("editor-file", Some("s-1"), 10)
            .await
            .unwrap();
        assert_eq!(rollup.len(), 2);
        // b.ts was inserted last → most recent → first.
        assert_eq!(rollup[0].key, "b.ts");
        assert_eq!(rollup[0].count, 1);
        assert_eq!(rollup[1].key, "a.ts");
        assert_eq!(rollup[1].count, 2);
        for r in &rollup {
            assert_eq!(r.kind, "editor-file");
        }

        // Without stream filter: c.ts also shows up.
        let global = store
            .list_recent_rollup("editor-file", None, 10)
            .await
            .unwrap();
        assert_eq!(global.len(), 3);
    }

    #[tokio::test]
    async fn usage_rollup_drops_rows_with_no_extractable_key() {
        let store = SqliteUsageStore::new(Database::in_memory());
        store
            .record("editor-file", serde_json::json!({"streamId": "s-1"}))
            .await
            .unwrap();
        store
            .record(
                "editor-file",
                serde_json::json!({"path": "real.ts", "streamId": "s-1"}),
            )
            .await
            .unwrap();
        let rollup = store
            .list_recent_rollup("editor-file", Some("s-1"), 10)
            .await
            .unwrap();
        assert_eq!(rollup.len(), 1);
        assert_eq!(rollup[0].key, "real.ts");
    }

    #[tokio::test]
    async fn code_quality_scan_lifecycle() {
        let store = SqliteCodeQualityStore::new(Database::in_memory());
        let id = store.create_scan("metrics", "workspace").await.unwrap();
        store
            .append_finding(
                id,
                CodeQualityFinding {
                    id: 0,
                    scan_id: id,
                    path: "src/main.rs".into(),
                    start_line: 10,
                    end_line: 50,
                    kind: "complexity".into(),
                    metric_value: 14.0,
                    extra_json: None,
                },
            )
            .await
            .unwrap();
        store
            .finish_scan(id, CodeQualityScanStatus::Done, None)
            .await
            .unwrap();
        let scans = store.list_scans(10).await.unwrap();
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].status, CodeQualityScanStatus::Done);
        let findings = store.list_findings(id).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].metric_value, 14.0);
    }

    #[tokio::test]
    async fn snapshot_capture_then_list() {
        let db = Database::in_memory();
        seed_stream(&db, "s-test");
        let store = SqliteSnapshotStore::new(db);
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: StreamId::from("s-test"),
                path: "src/foo.rs".into(),
                blob_hash: Some("abc".into()),
                size_bytes: 42,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: None,
                mtime_ms: None,
            })
            .await
            .unwrap();
        let list = store.list_for_path("src/foo.rs").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].size_bytes, 42);
    }

    #[tokio::test]
    async fn stats_for_snapshot_classifies_created_modified_deleted() {
        let db = Database::in_memory();
        seed_stream(&db, "s-test");
        let store = SqliteSnapshotStore::new(db);
        let stream = StreamId::from("s-test");

        // Parent 1: baseline of three paths.
        let p1 = store.create_snapshot(stream.clone()).await.unwrap();
        for path in ["a.txt", "b.txt", "c.txt"] {
            store
                .capture(FileSnapshot {
                    id: 0,
                    stream_id: stream.clone(),
                    path: path.into(),
                    blob_hash: Some(format!("h-{path}-v1")),
                    size_bytes: 10,
                    captured_at: Timestamp::now(),
                    oversize: false,
                    snapshot_id: Some(p1),
                    mtime_ms: None,
                })
                .await
                .unwrap();
        }

        // Parent 2: a modified, b unchanged-but-recaptured (modified by
        // the rule, since the row exists at all), c deleted, d created.
        let p2 = store.create_snapshot(stream.clone()).await.unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "a.txt".into(),
                blob_hash: Some("h-a.txt-v2".into()),
                size_bytes: 20,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p2),
                mtime_ms: None,
            })
            .await
            .unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "c.txt".into(),
                blob_hash: None,
                size_bytes: 0,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p2),
                mtime_ms: None,
            })
            .await
            .unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "d.txt".into(),
                blob_hash: Some("h-d.txt-v1".into()),
                size_bytes: 5,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p2),
                mtime_ms: None,
            })
            .await
            .unwrap();

        let summary = store.stats_for_snapshot(p2).await.unwrap();
        assert_eq!(summary.created, 1, "d.txt is new");
        assert_eq!(summary.modified, 1, "a.txt had a prior hash");
        assert_eq!(summary.deleted, 1, "c.txt has no blob");
        assert_eq!(summary.total, 3);

        // p1 itself: three created rows, nothing else.
        let p1_summary = store.stats_for_snapshot(p1).await.unwrap();
        assert_eq!(p1_summary.created, 3);
        assert_eq!(p1_summary.modified, 0);
        assert_eq!(p1_summary.deleted, 0);
        assert_eq!(p1_summary.total, 3);
    }

    #[tokio::test]
    async fn list_changes_for_snapshot_carries_status_and_prior_id() {
        let db = Database::in_memory();
        seed_stream(&db, "s-test");
        let store = SqliteSnapshotStore::new(db);
        let stream = StreamId::from("s-test");

        // p1: a and b baselined.
        let p1 = store.create_snapshot(stream.clone()).await.unwrap();
        let a1 = store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "a.txt".into(),
                blob_hash: Some("h-a-v1".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p1),
                mtime_ms: None,
            })
            .await
            .unwrap();
        store
            .capture(FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "b.txt".into(),
                blob_hash: Some("h-b-v1".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p1),
                mtime_ms: None,
            })
            .await
            .unwrap();

        // p2: a modified, c added, b deleted.
        let p2 = store.create_snapshot(stream.clone()).await.unwrap();
        for snap in [
            FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "a.txt".into(),
                blob_hash: Some("h-a-v2".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p2),
                mtime_ms: None,
            },
            FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "c.txt".into(),
                blob_hash: Some("h-c-v1".into()),
                size_bytes: 1,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p2),
                mtime_ms: None,
            },
            FileSnapshot {
                id: 0,
                stream_id: stream.clone(),
                path: "b.txt".into(),
                blob_hash: None,
                size_bytes: 0,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(p2),
                mtime_ms: None,
            },
        ] {
            store.capture(snap).await.unwrap();
        }

        let entries = store.list_changes_for_snapshot(p2).await.unwrap();
        let by_path: std::collections::HashMap<_, _> =
            entries.iter().map(|e| (e.path.clone(), e)).collect();
        assert_eq!(by_path["a.txt"].status, "modified");
        assert_eq!(by_path["a.txt"].prior_file_id, Some(a1));
        assert_eq!(by_path["c.txt"].status, "added");
        assert_eq!(by_path["c.txt"].prior_file_id, None);
        assert_eq!(by_path["b.txt"].status, "deleted");
        assert!(by_path["b.txt"].prior_file_id.is_some());
    }

    fn seed_stream(db: &Database, id: &str) {
        let conn = db.conn().unwrap();
        conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES (?1, 'primary', 't', 'main', 'refs/heads/main', 'main', '/r', '2026-01-01', '2026-01-01')",
            params![id],
        ).unwrap();
    }
}

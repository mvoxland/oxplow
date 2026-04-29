//! Wiki-note metadata + FTS5-backed body search.
//!
//! Note body lives on disk at `.oxplow/notes/<slug>.md`. This store
//! holds the metadata row + an FTS5 search index synced from
//! the on-disk body via `resync`.

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, Timestamp};

use crate::database::Database;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WikiNote {
    pub slug: String,
    pub title: String,
    pub body_path: String,
    pub body_excerpt: String,
    pub body_size_bytes: i64,
    pub file_refs: Vec<String>,
    pub related_notes: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WikiNoteSearchHit {
    pub slug: String,
    pub title: String,
    pub snippet: String,
    pub updated_at: Timestamp,
}

#[derive(Clone)]
pub struct SqliteWikiNoteStore {
    db: Database,
}

impl SqliteWikiNoteStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts).unwrap().trim_matches('"').to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<WikiNote> {
    let slug: String = row.get("slug")?;
    let title: String = row.get("title")?;
    let body_path: String = row.get("body_path")?;
    let body_excerpt: String = row.get("body_excerpt")?;
    let body_size_bytes: i64 = row.get("body_size_bytes")?;
    let file_refs_json: String = row.get("file_refs_json")?;
    let related_notes_json: String = row.get("related_notes_json")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let file_refs: Vec<String> = serde_json::from_str(&file_refs_json).unwrap_or_default();
    let related_notes: Vec<String> = serde_json::from_str(&related_notes_json).unwrap_or_default();
    Ok(WikiNote {
        slug,
        title,
        body_path,
        body_excerpt,
        body_size_bytes,
        file_refs,
        related_notes,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
    })
}

impl SqliteWikiNoteStore {
    pub async fn list(&self) -> Result<Vec<WikiNote>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT * FROM wiki_note ORDER BY updated_at DESC")?;
                let rows = stmt.query_map([], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    pub async fn get(&self, slug: &str) -> Result<Option<WikiNote>, DomainError> {
        let db = self.db.clone();
        let slug = slug.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT * FROM wiki_note WHERE slug = ?1")?;
                let mut rows = stmt.query_map(params![slug], row_to_note)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
        })
        .await
        .unwrap()
    }

    pub async fn upsert(&self, note: &WikiNote) -> Result<(), DomainError> {
        let db = self.db.clone();
        let note = note.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let file_refs_json = serde_json::to_string(&note.file_refs)
                    .unwrap_or_else(|_| "[]".to_string());
                let related_notes_json = serde_json::to_string(&note.related_notes)
                    .unwrap_or_else(|_| "[]".to_string());
                conn.execute(
                    "INSERT INTO wiki_note (
                        slug, title, body_path, body_excerpt, body_size_bytes,
                        file_refs_json, related_notes_json, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(slug) DO UPDATE SET
                        title = excluded.title,
                        body_path = excluded.body_path,
                        body_excerpt = excluded.body_excerpt,
                        body_size_bytes = excluded.body_size_bytes,
                        file_refs_json = excluded.file_refs_json,
                        related_notes_json = excluded.related_notes_json,
                        updated_at = excluded.updated_at",
                    params![
                        note.slug,
                        note.title,
                        note.body_path,
                        note.body_excerpt,
                        note.body_size_bytes,
                        file_refs_json,
                        related_notes_json,
                        ts_to_string(note.created_at),
                        ts_to_string(note.updated_at),
                    ],
                )?;
                // FTS5 mirror.
                conn.execute(
                    "DELETE FROM wiki_note_fts WHERE slug = ?1",
                    params![note.slug],
                )?;
                conn.execute(
                    "INSERT INTO wiki_note_fts (slug, title, body_excerpt) VALUES (?1, ?2, ?3)",
                    params![note.slug, note.title, note.body_excerpt],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    pub async fn delete(&self, slug: &str) -> Result<(), DomainError> {
        let db = self.db.clone();
        let slug = slug.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute("DELETE FROM wiki_note WHERE slug = ?1", params![slug])?;
                conn.execute("DELETE FROM wiki_note_fts WHERE slug = ?1", params![slug])?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    /// FTS5-backed full-text search over the body excerpt + title.
    pub async fn search_bodies(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiNoteSearchHit>, DomainError> {
        let db = self.db.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT n.slug, n.title, snippet(wiki_note_fts, 2, '<b>', '</b>', '…', 12) AS snippet,
                            n.updated_at
                     FROM wiki_note_fts f
                     JOIN wiki_note n ON n.slug = f.slug
                     WHERE wiki_note_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![query, limit as i64], |row| {
                    let slug: String = row.get(0)?;
                    let title: String = row.get(1)?;
                    let snippet: String = row.get(2)?;
                    let updated_at: String = row.get(3)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(WikiNoteSearchHit {
                        slug,
                        title,
                        snippet,
                        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    /// Glob-by-title for the lighter search_notes MCP tool.
    pub async fn search_titles(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiNote>, DomainError> {
        let db = self.db.clone();
        let pattern = format!("%{}%", query);
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM wiki_note WHERE title LIKE ?1 OR slug LIKE ?1 \
                     ORDER BY updated_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![pattern, limit as i64], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }
}

#[async_trait]
pub trait WikiNoteStore: Send + Sync {
    async fn list(&self) -> Result<Vec<WikiNote>, DomainError>;
    async fn get(&self, slug: &str) -> Result<Option<WikiNote>, DomainError>;
    async fn upsert(&self, note: &WikiNote) -> Result<(), DomainError>;
    async fn delete(&self, slug: &str) -> Result<(), DomainError>;
    async fn search_bodies(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiNoteSearchHit>, DomainError>;
    async fn search_titles(&self, query: &str, limit: usize) -> Result<Vec<WikiNote>, DomainError>;
}

#[async_trait]
impl WikiNoteStore for SqliteWikiNoteStore {
    async fn list(&self) -> Result<Vec<WikiNote>, DomainError> {
        SqliteWikiNoteStore::list(self).await
    }
    async fn get(&self, slug: &str) -> Result<Option<WikiNote>, DomainError> {
        SqliteWikiNoteStore::get(self, slug).await
    }
    async fn upsert(&self, note: &WikiNote) -> Result<(), DomainError> {
        SqliteWikiNoteStore::upsert(self, note).await
    }
    async fn delete(&self, slug: &str) -> Result<(), DomainError> {
        SqliteWikiNoteStore::delete(self, slug).await
    }
    async fn search_bodies(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiNoteSearchHit>, DomainError> {
        SqliteWikiNoteStore::search_bodies(self, query, limit).await
    }
    async fn search_titles(&self, query: &str, limit: usize) -> Result<Vec<WikiNote>, DomainError> {
        SqliteWikiNoteStore::search_titles(self, query, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn note(slug: &str, title: &str, excerpt: &str) -> WikiNote {
        WikiNote {
            slug: slug.into(),
            title: title.into(),
            body_path: format!(".oxplow/notes/{slug}.md"),
            body_excerpt: excerpt.into(),
            body_size_bytes: excerpt.len() as i64,
            file_refs: vec![],
            related_notes: vec![],
            created_at: now(),
            updated_at: now(),
        }
    }

    #[tokio::test]
    async fn upsert_get_round_trips() {
        let store = SqliteWikiNoteStore::new(Database::in_memory());
        let n = note("hello", "Hello world", "the quick brown fox");
        store.upsert(&n).await.unwrap();
        let got = store.get("hello").await.unwrap().unwrap();
        assert_eq!(got, n);
    }

    #[tokio::test]
    async fn fts_finds_body_terms() {
        let store = SqliteWikiNoteStore::new(Database::in_memory());
        store
            .upsert(&note("a", "Cats and dogs", "cats are great pets"))
            .await
            .unwrap();
        store
            .upsert(&note("b", "Lizards", "reptilian friends"))
            .await
            .unwrap();
        let hits = store.search_bodies("cats", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "a");
    }

    #[tokio::test]
    async fn search_titles_glob() {
        let store = SqliteWikiNoteStore::new(Database::in_memory());
        store.upsert(&note("a", "Streams", "")).await.unwrap();
        store.upsert(&note("b", "Threads", "")).await.unwrap();
        let hits = store.search_titles("Thread", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "b");
    }

    #[tokio::test]
    async fn delete_clears_fts_too() {
        let store = SqliteWikiNoteStore::new(Database::in_memory());
        store.upsert(&note("a", "x", "find me")).await.unwrap();
        store.delete("a").await.unwrap();
        assert!(store.search_bodies("me", 10).await.unwrap().is_empty());
    }
}

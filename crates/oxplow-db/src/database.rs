use std::path::Path;
use std::sync::Arc;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use tracing::info;

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// Pooled SQLite connection used by every store impl in this crate.
///
/// Constructed once at app startup; `Arc` it and hand it to each
/// store. Connections are obtained via `pool.get()`; DB calls run
/// inside `tokio::task::spawn_blocking` from the service layer so
/// the synchronous rusqlite API doesn't block the async runtime.
#[derive(Clone)]
pub struct Database {
    pool: Arc<Pool<SqliteConnectionManager>>,
}

impl Database {
    /// Open (or create) the SQLite file at `path` and apply migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbInitError> {
        let manager = SqliteConnectionManager::file(path.as_ref()).with_init(|c| {
            c.pragma_update(None, "journal_mode", "WAL")?;
            c.pragma_update(None, "foreign_keys", "ON")?;
            c.pragma_update(None, "synchronous", "NORMAL")?;
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(8)
            .build(manager)
            .map_err(DbInitError::Pool)?;

        let mut conn = pool.get().map_err(DbInitError::Pool)?;
        embedded::migrations::runner()
            .run(&mut *conn)
            .map_err(|e| DbInitError::Migration(e.to_string()))?;
        info!("oxplow db opened at {}", path.as_ref().display());

        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// In-memory DB for tests. Each call returns a fresh DB.
    ///
    /// Public so other crates' tests can build a Services graph
    /// without needing a tempfile.
    pub fn in_memory() -> Self {
        let manager = SqliteConnectionManager::memory().with_init(|c| {
            c.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        });
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        let mut conn = pool.get().unwrap();
        embedded::migrations::runner().run(&mut *conn).unwrap();
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Borrow a connection from the pool. Most stores should call this
    /// inside a `spawn_blocking` so the synchronous rusqlite API doesn't
    /// stall the tokio runtime.
    pub(crate) fn conn(&self) -> Result<r2d2::PooledConnection<SqliteConnectionManager>, r2d2::Error> {
        self.pool.get()
    }

    /// Run a closure with a borrowed connection. Pure convenience
    /// wrapper that maps pool errors into `oxplow_domain::DomainError`.
    pub(crate) fn with_conn<R>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R>,
    ) -> Result<R, oxplow_domain::DomainError> {
        let conn = self
            .conn()
            .map_err(|e| oxplow_domain::DomainError::Invalid(format!("pool: {e}")))?;
        f(&conn).map_err(|e| oxplow_domain::DomainError::Invalid(format!("sql: {e}")))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DbInitError {
    #[error("connection pool: {0}")]
    Pool(r2d2::Error),
    #[error("migration: {0}")]
    Migration(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_db_runs_migrations() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        // Sanity check: the streams table exists after migrations.
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='streams'")
            .unwrap();
        let row: String = stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(row, "streams");
    }

    #[test]
    fn runtime_state_seeded() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runtime_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}

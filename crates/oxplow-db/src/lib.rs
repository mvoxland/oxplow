//! SQLite persistence layer for oxplow.
//!
//! Implements the store traits defined in `oxplow-domain` against a
//! `rusqlite` connection pool. Migrations live in `migrations/` as
//! plain SQL and are applied at startup via `refinery`.

mod database;
mod stream_store;
mod thread_store;
mod work_item_store;

pub use database::{Database, DbInitError};
pub use stream_store::SqliteStreamStore;
pub use thread_store::SqliteThreadStore;
pub use work_item_store::SqliteWorkItemStore;

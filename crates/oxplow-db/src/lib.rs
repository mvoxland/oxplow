//! SQLite persistence layer for oxplow.
//!
//! Implements the store traits defined in `oxplow-domain` against a
//! `rusqlite` connection pool. Migrations live in `migrations/` as
//! plain SQL and are applied at startup via `refinery`.

pub mod analytics_stores;
mod database;
mod stream_store;
mod thread_store;
pub mod wiki_note_store;
mod work_item_store;
mod work_satellite;

pub use analytics_stores::{
    CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus, FileSnapshot, PageVisit,
    PageVisitStore, SqliteCodeQualityStore, SqlitePageVisitStore, SqliteSnapshotStore,
    SqliteUsageStore, UsageEvent,
};
pub use database::{Database, DbInitError};
pub use stream_store::SqliteStreamStore;
pub use thread_store::SqliteThreadStore;
pub use wiki_note_store::{SqliteWikiNoteStore, WikiNote, WikiNoteSearchHit, WikiNoteStore};
pub use work_item_store::SqliteWorkItemStore;
pub use work_satellite::{SqliteWorkItemEventStore, SqliteWorkItemLinkStore, SqliteWorkNoteStore};

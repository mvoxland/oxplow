//! fs watcher that keeps the `wiki_note` rows in sync with the
//! `.oxplow/notes/` markdown files.
//!
//! Mirrors `src/git/notes-watch.ts` from main: an initial scan on
//! start, then debounced per-slug re-syncs on file change. Wraps
//! [`oxplow_fs_watch::FsWatcher`] for the debouncing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oxplow_db::SqliteWikiNoteStore;
use oxplow_fs_watch::FsWatcher;
use tracing::{info, warn};

use crate::events::{EventBus, OxplowEvent};
use crate::wiki_notes;

/// Spawn a wiki-note watcher. Holding the returned struct keeps the
/// watcher alive; dropping it cancels the OS handles + the relay
/// task (channel close).
pub struct WikiNotesWatcher {
    _watcher: FsWatcher,
}

impl WikiNotesWatcher {
    /// Boot — runs the initial scan synchronously, then attaches the
    /// debounced fs watcher. Errors during scan are logged but don't
    /// prevent the watcher from starting.
    pub async fn spawn(
        project_dir: PathBuf,
        store: Arc<SqliteWikiNoteStore>,
        events: EventBus,
    ) -> Option<Self> {
        let dir = wiki_notes::notes_dir(&project_dir);
        std::fs::create_dir_all(&dir).ok();

        if let Err(err) = wiki_notes::scan_and_sync_all(&project_dir, &store).await {
            warn!(?err, "wiki notes initial scan failed");
        } else {
            info!(dir = %dir.display(), "wiki notes initial scan complete");
        }

        let watcher = match FsWatcher::watch(&dir, Duration::from_millis(200)) {
            Ok(w) => w,
            Err(err) => {
                warn!(?err, "wiki notes watcher failed to start");
                return None;
            }
        };
        let mut rx = watcher.subscribe();

        let project_dir_for_loop = project_dir.clone();
        let store_for_loop = store.clone();
        let events_for_loop = events.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        if evt.path.extension().and_then(|s| s.to_str()) != Some("md") {
                            continue;
                        }
                        let Some(slug) = evt.path.file_stem().and_then(|s| s.to_str()) else {
                            continue;
                        };
                        if let Err(err) = wiki_notes::sync_from_disk(
                            &project_dir_for_loop,
                            &store_for_loop,
                            slug,
                        )
                        .await
                        {
                            warn!(slug, ?err, "wiki note resync failed");
                            continue;
                        }
                        events_for_loop.emit(OxplowEvent::WikiNotesChanged);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "wiki notes watcher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Some(Self { _watcher: watcher })
    }
}

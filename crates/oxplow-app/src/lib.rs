//! Application services / use-cases layer.
//!
//! Constructs the dependency graph: Database → store impls →
//! services. The Tauri command crate and the MCP crate both call into
//! this layer; they never reach into infrastructure crates directly.
//!
//! Held inside `Arc<Services>` and registered as Tauri state. Methods
//! on `Services` are the high-level "use cases" the IPC layer calls.

pub mod agent_command;
pub mod agent_pane;
pub mod agent_prompt;
pub mod agent_status_derive;
pub mod blob_store;
pub mod code_quality_runner;
pub mod background_task;
pub mod config_service;
pub mod diagnostics;
pub mod events;
pub mod followup;
pub mod git_service;
pub mod hook_ingest;
pub mod wiki_notes;
pub mod wiki_notes_watch;
pub mod lsp_clients;
pub mod lsp_sessions;
pub mod terminal_sessions;
pub mod recovery;
pub mod snapshot_capture;
pub mod thread_runtime;
pub mod work_item_service;
pub mod workspace_watch;

pub use events::{
    CodeQualityScanPhase, EventBus, OxplowEvent, SnapshotSourceKind, WorkspaceChangeKind,
};
pub use oxplow_lsp::{LspError, LspProxy};
pub use hook_ingest::{HookEnvelope, HookIngestError, HookIngestService};
pub use work_item_service::{
    BacklogState, CreateWorkItemInput, UpdateWorkItemChanges, WorkItemService,
    WorkItemServiceError,
};

use std::path::PathBuf;
use std::sync::Arc;

pub use background_task::{
    BackgroundTask, BackgroundTaskChange, BackgroundTaskChangeKind, BackgroundTaskKind,
    BackgroundTaskStatus, BackgroundTaskStore, StartInput,
};
pub use followup::{Followup, FollowupStore};

use thiserror::Error;
use tracing::info;

use std::sync::RwLock;

use oxplow_config::OxplowConfig;
use oxplow_db::{
    Database, SqliteAgentTurnStore, SqliteCodeQualityStore, SqlitePageVisitStore,
    SqliteSnapshotStore, SqliteStreamStore, SqliteThreadStore, SqliteUsageStore,
    SqliteWikiNoteStore, SqliteWikiNoteThreadUpdateStore, SqliteWorkItemEffortStore,
    SqliteWorkItemEventStore, SqliteWorkItemLinkStore, SqliteWorkItemStore, SqliteWorkNoteStore,
};
use oxplow_domain::stores::{AgentStatusStore, HookEventStore};
use oxplow_session::{StreamService, ThreadService, WorkspaceLayout};

#[derive(Debug, Error)]
pub enum AppInitError {
    #[error("config: {0}")]
    Config(#[from] oxplow_config::ConfigError),
    #[error("db: {0}")]
    Db(#[from] oxplow_db::DbInitError),
    #[error("session: {0}")]
    Session(#[from] oxplow_session::SessionError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Layout of the on-disk state for one project. Lives under
/// `<project>/.oxplow/`.
pub struct AppLayout {
    pub project_dir: PathBuf,
    pub state_dir: PathBuf,
    pub state_db_path: PathBuf,
}

impl AppLayout {
    pub fn for_project(project_dir: impl Into<PathBuf>) -> Self {
        let project_dir = project_dir.into();
        let state_dir = project_dir.join(".oxplow");
        let state_db_path = state_dir.join("state.sqlite");
        Self {
            project_dir,
            state_dir,
            state_db_path,
        }
    }
}

/// All the long-lived services oxplow needs to serve a UI.
///
/// Registered with Tauri as `tauri::State<Arc<Services>>`, so the
/// renderer never clones `Services` directly — every reader bumps the
/// `Arc` refcount instead. The inner pieces (PtyManager, EventBus,
/// SqliteSnapshotStore, etc.) all derive `Clone` and route through
/// shared owner tasks via `mpsc`/`broadcast`, so even an accidental
/// `Services.clone()` doesn't spawn a duplicate runtime — it just
/// hands out another sender into the same backing task.
pub struct Services {
    pub config: Arc<RwLock<OxplowConfig>>,
    pub db: Database,
    pub layout: AppLayout,
    pub streams: StreamService,
    pub threads: ThreadService,
    pub work_items: WorkItemService,
    pub stream_store: Arc<SqliteStreamStore>,
    pub thread_store: Arc<SqliteThreadStore>,
    pub work_item_store: Arc<SqliteWorkItemStore>,
    pub work_note_store: Arc<SqliteWorkNoteStore>,
    pub work_item_link_store: Arc<SqliteWorkItemLinkStore>,
    pub work_item_event_store: Arc<SqliteWorkItemEventStore>,
    pub wiki_note_store: Arc<SqliteWikiNoteStore>,
    pub page_visit_store: Arc<SqlitePageVisitStore>,
    pub usage_store: Arc<SqliteUsageStore>,
    pub code_quality_store: Arc<SqliteCodeQualityStore>,
    pub snapshot_store: Arc<SqliteSnapshotStore>,
    pub hook_event_store: Arc<dyn HookEventStore>,
    pub agent_status_store: Arc<dyn AgentStatusStore>,
    pub agent_turn_store: Arc<SqliteAgentTurnStore>,
    /// Backing in-memory state for hook events + agent status. Both
    /// `hook_event_store` and `agent_status_store` are trait-object
    /// views of this same registry — keep the concrete handle around
    /// for code that wants to bypass the trait surfaces.
    pub thread_runtime: Arc<thread_runtime::ThreadRuntimeRegistry>,
    pub effort_store: Arc<SqliteWorkItemEffortStore>,
    pub wiki_note_thread_updates: Arc<SqliteWikiNoteThreadUpdateStore>,
    pub hook_ingest: HookIngestService,
    pub background_tasks: BackgroundTaskStore,
    pub followups: FollowupStore,
    pub pty: oxplow_pty::PtyManager,
    pub tmux: Arc<dyn oxplow_tmux::TmuxRunner>,
    pub agent_panes: agent_pane::AgentPaneService,
    pub blobs: blob_store::BlobStore,
    pub lsp_sessions: lsp_sessions::LspSessionManager,
    pub lsp_clients: lsp_clients::LspClientRegistry,
    pub terminal_sessions: terminal_sessions::TerminalSessionRegistry,
    pub recovery: recovery::RecoveryService,
    pub events: EventBus,
    /// Singleton git access surface — every read of git state and
    /// every mutating git op routes through here so we can layer
    /// caching in one place. See `git_service.rs`.
    pub git: Arc<git_service::GitService>,
    /// Per-thread cursor for the rail's "Recently finished" section.
    /// Entries whose timestamp is `<= cursor` are filtered out. Keyed
    /// by thread id; entries with no thread (global view) live under
    /// the empty string. In-memory only — clearing the section is a UX
    /// gesture, not a destructive op, and re-appearing after a restart
    /// is fine.
    pub finished_cleared_at:
        Arc<RwLock<std::collections::HashMap<String, oxplow_domain::Timestamp>>>,
}

impl Services {
    /// Bootstrap. Run once at app startup.
    pub fn boot(layout: AppLayout) -> Result<Self, AppInitError> {
        std::fs::create_dir_all(&layout.state_dir)?;

        let config = oxplow_config::load_project_config(&layout.project_dir)?;
        info!(project = %layout.project_dir.display(), agent = ?config.agent, "config loaded");

        let db = Database::open(&layout.state_db_path)?;

        let stream_store = Arc::new(SqliteStreamStore::new(db.clone()));
        let thread_store = Arc::new(SqliteThreadStore::new(db.clone()));
        let work_item_store = Arc::new(SqliteWorkItemStore::new(db.clone()));
        let work_note_store = Arc::new(SqliteWorkNoteStore::new(db.clone()));
        let work_item_link_store = Arc::new(SqliteWorkItemLinkStore::new(db.clone()));
        let work_item_event_store = Arc::new(SqliteWorkItemEventStore::new(db.clone()));
        let wiki_note_store = Arc::new(SqliteWikiNoteStore::new(db.clone()));
        let page_visit_store = Arc::new(SqlitePageVisitStore::new(db.clone()));
        let usage_store = Arc::new(SqliteUsageStore::new(db.clone()));
        let code_quality_store = Arc::new(SqliteCodeQualityStore::new(db.clone()));
        let snapshot_store = Arc::new(SqliteSnapshotStore::new(db.clone()));
        let thread_runtime =
            Arc::new(thread_runtime::ThreadRuntimeRegistry::with_default_capacity());
        let hook_event_store: Arc<dyn HookEventStore> = thread_runtime.clone();
        let agent_status_store: Arc<dyn AgentStatusStore> = thread_runtime.clone();
        let agent_turn_store = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let effort_store = Arc::new(SqliteWorkItemEffortStore::new(db.clone()));
        let wiki_note_thread_updates =
            Arc::new(SqliteWikiNoteThreadUpdateStore::new(db.clone()));

        let workspace_layout = WorkspaceLayout::for_project(&layout.project_dir);
        let streams = StreamService::new(workspace_layout, stream_store.clone(), thread_store.clone());
        let threads = ThreadService::new(thread_store.clone());
        let work_items = WorkItemService::new(work_item_store.clone());
        let event_bus = EventBus::new();
        let hook_ingest = HookIngestService::new(
            hook_event_store.clone(),
            agent_status_store.clone(),
            agent_turn_store.clone(),
            event_bus.clone(),
        );
        let recovery_svc =
            recovery::RecoveryService::new(agent_turn_store.clone(), event_bus.clone());

        let pty = oxplow_pty::PtyManager::spawn();
        let tmux: Arc<dyn oxplow_tmux::TmuxRunner> = Arc::new(oxplow_tmux::SystemTmux::new());
        let agent_panes = agent_pane::AgentPaneService::new(tmux.clone());
        // Lazily-built per-(stream, language) LSP proxies. Spawn cost
        // is paid on first request, not at boot.
        let config_arc = Arc::new(RwLock::new(config));
        let lsp = lsp_sessions::LspSessionManager::new(config_arc.clone());
        let lsp_clients = lsp_clients::LspClientRegistry::new(config_arc.clone());
        let terminal_sessions =
            terminal_sessions::TerminalSessionRegistry::new(pty.clone(), tmux.clone());
        let blobs = blob_store::BlobStore::new(layout.state_dir.join("blobs"));
        let git = git_service::GitService::spawn(
            layout.project_dir.clone(),
            stream_store.clone(),
            event_bus.clone(),
        );

        Ok(Self {
            config: config_arc,
            db,
            layout,
            streams,
            threads,
            work_items,
            stream_store,
            thread_store,
            work_item_store,
            work_note_store,
            work_item_link_store,
            work_item_event_store,
            wiki_note_store,
            page_visit_store,
            usage_store,
            code_quality_store,
            snapshot_store,
            hook_event_store,
            agent_status_store,
            agent_turn_store,
            thread_runtime,
            effort_store,
            wiki_note_thread_updates,
            hook_ingest,
            background_tasks: BackgroundTaskStore::new(),
            followups: FollowupStore::new(),
            pty,
            tmux,
            agent_panes,
            blobs,
            lsp_sessions: lsp,
            lsp_clients,
            terminal_sessions,
            recovery: recovery_svc,
            events: event_bus,
            git,
            finished_cleared_at: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Test-only constructor with an in-memory DB. Useful for the
    /// IPC layer's smoke tests where we want a real Services without
    /// hitting the filesystem.
    pub fn in_memory(project_dir: impl Into<PathBuf>) -> Result<Self, AppInitError> {
        let project_dir = project_dir.into();
        let state_dir = project_dir.join(".oxplow");
        std::fs::create_dir_all(&state_dir)?;
        let layout = AppLayout {
            project_dir: project_dir.clone(),
            state_dir: state_dir.clone(),
            state_db_path: state_dir.join("state.sqlite"),
        };
        let config = oxplow_config::load_project_config(&project_dir)?;
        let db = Database::in_memory();
        let stream_store = Arc::new(SqliteStreamStore::new(db.clone()));
        let thread_store = Arc::new(SqliteThreadStore::new(db.clone()));
        let work_item_store = Arc::new(SqliteWorkItemStore::new(db.clone()));
        let work_note_store = Arc::new(SqliteWorkNoteStore::new(db.clone()));
        let work_item_link_store = Arc::new(SqliteWorkItemLinkStore::new(db.clone()));
        let work_item_event_store = Arc::new(SqliteWorkItemEventStore::new(db.clone()));
        let wiki_note_store = Arc::new(SqliteWikiNoteStore::new(db.clone()));
        let page_visit_store = Arc::new(SqlitePageVisitStore::new(db.clone()));
        let usage_store = Arc::new(SqliteUsageStore::new(db.clone()));
        let code_quality_store = Arc::new(SqliteCodeQualityStore::new(db.clone()));
        let snapshot_store = Arc::new(SqliteSnapshotStore::new(db.clone()));
        let thread_runtime =
            Arc::new(thread_runtime::ThreadRuntimeRegistry::with_default_capacity());
        let hook_event_store: Arc<dyn HookEventStore> = thread_runtime.clone();
        let agent_status_store: Arc<dyn AgentStatusStore> = thread_runtime.clone();
        let agent_turn_store = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let effort_store = Arc::new(SqliteWorkItemEffortStore::new(db.clone()));
        let wiki_note_thread_updates =
            Arc::new(SqliteWikiNoteThreadUpdateStore::new(db.clone()));
        let workspace_layout = WorkspaceLayout::for_project(&project_dir);
        let streams = StreamService::new(workspace_layout, stream_store.clone(), thread_store.clone());
        let threads = ThreadService::new(thread_store.clone());
        let work_items = WorkItemService::new(work_item_store.clone());
        let event_bus = EventBus::new();
        let hook_ingest = HookIngestService::new(
            hook_event_store.clone(),
            agent_status_store.clone(),
            agent_turn_store.clone(),
            event_bus.clone(),
        );
        let recovery_svc =
            recovery::RecoveryService::new(agent_turn_store.clone(), event_bus.clone());
        let pty = oxplow_pty::PtyManager::spawn();
        let tmux: Arc<dyn oxplow_tmux::TmuxRunner> = Arc::new(oxplow_tmux::SystemTmux::new());
        let agent_panes = agent_pane::AgentPaneService::new(tmux.clone());
        let config_arc = Arc::new(RwLock::new(config));
        let lsp = lsp_sessions::LspSessionManager::new(config_arc.clone());
        let lsp_clients = lsp_clients::LspClientRegistry::new(config_arc.clone());
        let terminal_sessions =
            terminal_sessions::TerminalSessionRegistry::new(pty.clone(), tmux.clone());
        let blobs = blob_store::BlobStore::new(layout.state_dir.join("blobs"));
        let git = git_service::GitService::spawn(
            layout.project_dir.clone(),
            stream_store.clone(),
            event_bus.clone(),
        );
        Ok(Self {
            config: config_arc,
            db,
            layout,
            streams,
            threads,
            work_items,
            stream_store,
            thread_store,
            work_item_store,
            work_note_store,
            work_item_link_store,
            work_item_event_store,
            wiki_note_store,
            page_visit_store,
            usage_store,
            code_quality_store,
            snapshot_store,
            hook_event_store,
            agent_status_store,
            agent_turn_store,
            thread_runtime,
            effort_store,
            wiki_note_thread_updates,
            hook_ingest,
            background_tasks: BackgroundTaskStore::new(),
            followups: FollowupStore::new(),
            pty,
            tmux,
            agent_panes,
            blobs,
            lsp_sessions: lsp,
            lsp_clients,
            terminal_sessions,
            recovery: recovery_svc,
            events: event_bus,
            git,
            finished_cleared_at: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn boot_creates_state_dir() {
        let project = tempdir().unwrap();
        // Init a git repo so session validation passes for any
        // future calls that go through StreamService.
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

        let layout = AppLayout::for_project(project.path());
        let services = Services::boot(layout).unwrap();
        assert!(services.layout.state_dir.exists());
        assert!(services.layout.state_db_path.exists());
    }

    #[tokio::test]
    async fn in_memory_does_not_touch_disk_db() {
        let project = tempdir().unwrap();
        let services = Services::in_memory(project.path()).unwrap();
        // The state dir is created (config load needs it for fallback
        // basename) but the DB is in-memory.
        assert!(services.layout.state_dir.exists());
        // Writing to db should be fine; the file path will not exist.
        assert!(!services.layout.state_db_path.exists());
    }
}

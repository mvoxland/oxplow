// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use oxplow_app::{AppLayout, BackgroundTaskKind, Services, StartInput};
use oxplow_tauri_ipc::{
    specta_builder, AppState, PluginRuntime, PluginRuntimeState, OXPLOW_EVENT_CHANNEL,
};
use tauri::Emitter;

fn main() {
    init_tracing();

    // Project root resolution. `tauri dev` runs the binary with cwd
    // set to `apps/desktop` (the package being built), which isn't a
    // git toplevel — `ensure_primary` would refuse it and the
    // renderer would see "no primary stream available". Honour
    // `OXPLOW_PROJECT_DIR` so the dev launcher can pin cwd to the
    // repo root; production launches via `./bin/oxplow` from the
    // repo root and just use cwd as before.
    let project_dir = std::env::var_os("OXPLOW_PROJECT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
    let layout = AppLayout::for_project(&project_dir);
    // Services::boot synchronously calls `tokio::spawn` (PtyManager owner
    // task), which requires an entered Tokio runtime. Tauri builds its
    // own runtime later, so we stand up a dedicated multi-thread runtime
    // here, enter it for the duration of boot, and leak the runtime so
    // background tasks keep running for the life of the process.
    let boot_runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime"),
    ));
    let _enter = boot_runtime.enter();
    let services = Services::boot(layout).expect("services boot");

    let state: AppState = Arc::new(services);
    let event_bus = state.events.clone();
    let lsp_clients = state.lsp_clients.clone();
    let terminal_sessions = state.terminal_sessions.clone();

    // Run daemon recovery — close any agent_turn rows that the
    // previous boot left open, reset agent_status rows from
    // Running/AwaitingUser to Stopped. Synchronous so the renderer
    // doesn't see stale state.
    let recovery = state.recovery.clone();
    tauri::async_runtime::block_on(async move {
        if let Err(e) = recovery.run().await {
            tracing::warn!(error = %e, "daemon recovery failed");
        }
    });

    // Ensure the project's primary stream (and its default thread)
    // exist. `StreamService::ensure_primary` itself seeds the
    // auto-generated thread, so a single call covers both invariants
    // — every stream owns ≥1 thread.
    let streams = state.streams.clone();
    boot_runtime.block_on(async move {
        match streams.ensure_primary().await {
            Ok(s) => tracing::info!(stream_id = %s.id, "primary stream ready"),
            Err(e) => tracing::warn!(error = %e, "ensure_primary failed at boot"),
        }
    });

    // Start the periodic file-snapshot capture loop against the
    // project root. Runs for the lifetime of the daemon; events
    // funnel into the file_snapshot table so cross-turn diffs are
    // available even before content-addressed blob storage lands.
    let snap_store = state.snapshot_store.clone();
    let project_dir = state.layout.project_dir.clone();
    let max_bytes = state
        .config
        .read()
        .map(|c| c.snapshot_max_file_bytes)
        .unwrap_or(5 * 1024 * 1024);
    let blobs = state.blobs.clone();
    oxplow_app::snapshot_capture::SnapshotCaptureService::new(
        snap_store,
        blobs,
        project_dir.clone(),
        None,
        max_bytes,
    )
    .with_events(event_bus.clone())
    .spawn();

    // Per-stream fs + .git/refs watchers — bridges file changes onto
    // the EventBus so the renderer's QuickOpen, project panel, history,
    // git dashboard, etc. refresh without polling. Held in a registry
    // for the life of the daemon; dropping it cancels every watcher.
    //
    // Pushed off the synchronous boot path: the initial cache walk that
    // notify_debouncer_full performs can take seconds on large
    // worktrees. Surfacing it as a BackgroundTask lets the renderer
    // paint while watchers settle.
    {
        let stream_service = state.streams.clone();
        let watch_bus = event_bus.clone();
        let watch_project_dir = project_dir.clone();
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::Git,
            label: "Starting workspace watchers".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        boot_runtime.spawn(async move {
            let registry = oxplow_app::workspace_watch::WorkspaceWatchRegistry::spawn(
                stream_service,
                watch_bus,
                watch_project_dir,
            )
            .await;
            Box::leak(Box::new(registry));
            bts.complete(&task_id, None);
        });
    }

    // Wiki notes watcher: keeps `wiki_page` rows in sync with
    // `.oxplow/wiki/<slug>.md` on disk (initial scan + debounced
    // re-syncs on change). Held alive for the life of the process.
    //
    // One-shot legacy migration runs synchronously before the watcher
    // spawns: earlier versions stored bodies under `.oxplow/notes/`,
    // and the rename to `.oxplow/wiki/` would otherwise leave
    // existing pages stranded.
    oxplow_app::wiki_pages::migrate_legacy_notes_dir(&state.layout.project_dir);
    {
        let wiki_store = state.wiki_page_store.clone();
        let wiki_page_refs = state.page_ref_store.clone();
        let wiki_dir = state.layout.project_dir.clone();
        let wiki_events = event_bus.clone();
        let bts = state.background_tasks.clone();
        let task = bts.start(StartInput {
            kind: BackgroundTaskKind::NotesResync,
            label: "Initial wiki notes scan".into(),
            ..Default::default()
        });
        let task_id = task.id.clone();
        boot_runtime.spawn(async move {
            if let Some(watcher) = oxplow_app::wiki_pages_watch::WikiPagesWatcher::spawn(
                wiki_dir,
                wiki_store,
                wiki_page_refs,
                wiki_events,
            )
            .await
            {
                Box::leak(Box::new(watcher));
            }
            bts.complete(&task_id, None);
        });
    }

    // Lightweight self-diagnostics: once a minute, log RSS + open
    // fds + stream count so a long-running process leaves a trail
    // we can correlate against system-wide weirdness. See
    // `crates/oxplow-app/src/diagnostics.rs`.
    {
        let streams = state.stream_store.clone();
        boot_runtime.spawn(async move {
            oxplow_app::diagnostics::spawn(streams);
        });
    }

    // Unified page-ref graph backfill: re-project every existing
    // task, link, effort, and finding into the `page_ref`
    // table. Idempotent — second-run touches the same rows. Wiki
    // bodies and recent commits are covered by their own watchers
    // (above + below).
    {
        let page_refs = state.page_ref_store.clone();
        let tasks = state.task_store.clone();
        let links = state.task_link_store.clone();
        let efforts = state.effort_store.clone();
        let findings = state.code_quality_store.clone();
        let notes = state.work_note_store.clone();
        boot_runtime.spawn(async move {
            let counts = oxplow_app::page_ref_backfill::run(
                page_refs, tasks, links, efforts, findings, notes,
            )
            .await;
            tracing::info!(?counts, "page-ref backfill done");
        });
    }

    // Commit indexer: walk the most-recent N commits at boot to
    // populate `(git-commit:<sha>) -> (file/task/wiki/finding)`
    // edges, then re-scan whenever git refs change. Idempotent —
    // already-indexed commits are skipped via a one-row probe.
    {
        let repo_path = state.layout.project_dir.clone();
        let page_refs = state.page_ref_store.clone();
        let mut rx = state.events.subscribe();
        boot_runtime.spawn(async move {
            let n = oxplow_app::commit_indexer::index_recent(
                &repo_path,
                &page_refs,
                oxplow_app::commit_indexer::DEFAULT_INDEX_DEPTH,
            )
            .await;
            tracing::info!(indexed = n, "commit indexer initial scan done");
            // Re-index on every refs change. The watcher debounces;
            // the indexer's own existence-probe makes the scan cheap
            // when nothing's new.
            loop {
                match rx.recv().await {
                    Ok(oxplow_app::events::OxplowEvent::GitRefsChanged { .. }) => {
                        let _ = oxplow_app::commit_indexer::index_recent(
                            &repo_path,
                            &page_refs,
                            oxplow_app::commit_indexer::DEFAULT_INDEX_DEPTH,
                        )
                        .await;
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // Boot the in-process control plane (axum server hosting the
    // plugin's HTTP hook receiver + the streamable-HTTP MCP transport).
    // The handle's URLs + token feed the per-spawn plugin writer in
    // terminal.rs; nothing here needs to keep the handle around — the
    // axum task is detached.
    let control_plane = boot_runtime
        .block_on(async { oxplow_control_plane::spawn(state.clone()).await })
        .expect("control plane boot");
    let plugin_runtime: PluginRuntimeState = Arc::new(PluginRuntime {
        hook_base_url: control_plane.hook_base_url(),
        mcp_endpoint_url: control_plane.mcp_endpoint_url(),
        hook_token: control_plane.hook_token.clone(),
    });

    let specta = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(specta.invoke_handler())
        .setup(move |app| {
            specta.mount_events(app);
            spawn_event_bridge(app.handle().clone(), event_bus.clone());
            spawn_lsp_event_bridge(app.handle().clone(), lsp_clients.clone());
            spawn_terminal_event_bridge(app.handle().clone(), terminal_sessions.clone());
            oxplow_tauri_ipc::commands::menu::install_menu_handler(app.handle());
            Ok(())
        })
        .manage(state)
        .manage(plugin_runtime)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Forwards every `OxplowEvent` from the in-process bus onto the
/// Tauri event channel so the renderer can `listen("oxplow:event", …)`.
/// Lagging subscribers (slow renderer) are tolerated — they'll see a
/// `RecvError::Lagged` and the renderer typically refetches on
/// reconnect rather than replaying every event.
fn spawn_event_bridge(app: tauri::AppHandle, bus: oxplow_app::EventBus) {
    tauri::async_runtime::spawn(async move {
        let mut rx = bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit(OXPLOW_EVENT_CHANNEL, &event) {
                        tracing::warn!(?err, "failed to emit oxplow event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Forwards every `LspBridgeEvent` from the LSP client registry onto
/// `lsp:event` for the renderer's lsp.ts module.
fn spawn_lsp_event_bridge(
    app: tauri::AppHandle,
    registry: oxplow_app::lsp_clients::LspClientRegistry,
) {
    tauri::async_runtime::spawn(async move {
        let mut rx = registry.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit("lsp:event", &event) {
                        tracing::warn!(?err, "failed to emit lsp event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "lsp event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Forwards every `TerminalBridgeEvent` from the terminal session
/// registry onto `terminal:event` for the renderer's TerminalPane.
fn spawn_terminal_event_bridge(
    app: tauri::AppHandle,
    registry: oxplow_app::terminal_sessions::TerminalSessionRegistry,
) {
    tauri::async_runtime::spawn(async move {
        let mut rx = registry.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit("terminal:event", &event) {
                        tracing::warn!(?err, "failed to emit terminal event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "terminal event bridge lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,oxplow_=debug"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

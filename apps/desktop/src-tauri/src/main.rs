// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use oxplow_app::{AppLayout, Services};
use oxplow_tauri_ipc::{specta_builder, AppState, OXPLOW_EVENT_CHANNEL};
use tauri::Emitter;

fn main() {
    init_tracing();

    let project_dir = std::env::current_dir().expect("current dir");
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
        project_dir,
        None,
        max_bytes,
    )
    .spawn();

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

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,oxplow_=debug"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

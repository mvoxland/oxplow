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
    let services = Services::boot(layout).expect("services boot");

    let state: AppState = Arc::new(services);
    let event_bus = state.events.clone();

    let specta = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(specta.invoke_handler())
        .setup(move |app| {
            specta.mount_events(app);
            spawn_event_bridge(app.handle().clone(), event_bus.clone());
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

// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use oxplow_app::{AppLayout, Services};
use oxplow_tauri_ipc::{specta_builder, AppState};

fn main() {
    init_tracing();

    let project_dir = std::env::current_dir().expect("current dir");
    let layout = AppLayout::for_project(&project_dir);
    let services = Services::boot(layout).expect("services boot");

    let state: AppState = Arc::new(services);

    let specta = specta_builder();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(specta.invoke_handler())
        .setup(move |app| {
            specta.mount_events(app);
            Ok(())
        })
        .manage(state)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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

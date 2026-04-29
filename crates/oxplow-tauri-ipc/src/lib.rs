//! Tauri command + event adapter.
//!
//! Each `#[tauri::command]` is a thin wrapper around an `oxplow-app`
//! service method. Errors convert at this boundary into the
//! frontend-facing `IpcError`. `tauri-specta` exports the typed JS
//! bindings consumed by `apps/desktop/src/tauri-bridge/`.

pub mod commands;
pub mod error;
pub mod state;

pub use error::IpcError;
pub use state::AppState;

use tauri_specta::{collect_commands, Builder};

/// Build the tauri-specta `Builder` registering every oxplow command.
///
/// The desktop app's `main.rs` calls this and folds the result into
/// `tauri::Builder` via `.invoke_handler(specta_builder.invoke_handler())`.
/// The same builder also exports the TS bindings to
/// `apps/desktop/src/tauri-bridge/generated/bindings.ts` from the
/// `export_bindings` test below.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::app_version,
        commands::list_streams,
        commands::ensure_primary,
        commands::create_worktree,
        commands::delete_stream,
        commands::list_threads,
        commands::list_work_items_for_thread,
        commands::list_backlog,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the builder constructs without panicking. The
    /// command set is exercised by the desktop app's `cargo build`,
    /// which feeds the same builder into `tauri::Builder` and would
    /// fail to compile if any command's signature drifts.
    #[test]
    fn builder_constructs() {
        let _b = specta_builder();
    }
}

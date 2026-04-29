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
        commands::open_external_url,
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

    /// Regenerate the TS bindings file the frontend imports.
    ///
    /// Runs as part of `cargo test`; CI must verify `git diff` is
    /// empty after `cargo test --workspace`. If a Rust command
    /// signature changes, this test re-emits bindings and the diff
    /// flags the drift before the PR merges.
    #[test]
    fn export_ts_bindings() {
        // Skip when CARGO_MANIFEST_DIR doesn't resolve; defensive
        // against unusual cargo setups.
        let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
            Ok(v) => v,
            Err(_) => return,
        };
        let workspace_root = std::path::Path::new(&manifest_dir)
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root");
        let target = workspace_root
            .join("apps/desktop/src/tauri-bridge/generated/bindings.ts");
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("create bridge dir");
        }
        let builder = specta_builder();
        builder
            .export(specta_typescript::Typescript::default(), &target)
            .expect("export bindings");
        // Verify the file was written and contains *something*. Drift
        // detection is the responsibility of a CI git-diff check.
        let metadata = std::fs::metadata(&target).expect("bindings written");
        assert!(metadata.len() > 0, "bindings file should not be empty");
    }
}

//! Application services / use-cases layer.
//!
//! Wires the infrastructure crates (db, git, fs-watch, pty, tmux, lsp,
//! config) and the runtime/session services into a single `Services`
//! struct. The Tauri command crate and the MCP crate both call into
//! this layer; they never reach into infrastructure crates directly.

/// The orchestration container constructed once at startup.
///
/// Held inside `Arc<Services>` and registered as Tauri state. Methods
/// on `Services` are the high-level "use cases" the IPC layer calls.
#[derive(Default)]
pub struct Services {
    // Fields land as their owning crates come online.
}

impl Services {
    pub fn new() -> Self {
        Self::default()
    }
}

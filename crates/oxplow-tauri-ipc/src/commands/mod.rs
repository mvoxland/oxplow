//! Tauri command surface — split by area.
//!
//! Adding a new command: drop it in the relevant submodule, then
//! add the function name to `specta_builder()`'s `collect_commands![]`
//! list in `lib.rs`. The TS bindings regenerate on the next
//! `cargo test`.

pub mod agent_panes;
pub mod app;
pub mod backlog;
pub mod background;
pub mod branch;
pub mod code_quality;
pub mod config;
pub mod effort;
pub mod followup;
pub mod git;
pub mod hooks;
pub mod log;
pub mod notes;
pub mod page_visit;
pub mod snapshot;
pub mod streams;
pub mod threads;
pub mod usage;
pub mod webview;
pub mod wiki;
pub mod work_items;
pub mod workspace;

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

pub use oxplow_app::OxplowEvent;

/// Stable event channel name used by the renderer's `listen` calls.
/// Payload is `OxplowEvent` JSON.
pub const OXPLOW_EVENT_CHANNEL: &str = "oxplow:event";

/// Build the tauri-specta `Builder` registering every oxplow command.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        // app
        commands::app::app_version,
        commands::app::ping,
        // streams
        commands::streams::list_streams,
        commands::streams::ensure_primary,
        commands::streams::create_worktree,
        commands::streams::delete_stream,
        commands::streams::get_primary_stream,
        commands::streams::get_current_stream,
        commands::streams::switch_stream,
        commands::streams::rename_stream,
        // threads
        commands::threads::list_threads,
        commands::threads::get_thread,
        commands::threads::upsert_thread,
        commands::threads::delete_thread,
        commands::threads::create_thread,
        commands::threads::rename_thread,
        commands::threads::set_thread_prompt,
        commands::threads::promote_thread,
        commands::threads::close_thread,
        commands::threads::reopen_thread,
        commands::threads::list_closed_threads,
        commands::threads::reorder_thread_queue,
        commands::threads::get_selected_thread,
        commands::threads::select_thread,
        // work items
        commands::work_items::list_work_items_for_thread,
        commands::work_items::get_work_item,
        commands::work_items::upsert_work_item,
        commands::work_items::delete_work_item,
        commands::work_items::create_work_item,
        commands::work_items::update_work_item,
        commands::work_items::reorder_work_items,
        commands::work_items::move_work_item,
        // backlog
        commands::backlog::list_backlog,
        commands::backlog::get_backlog_state,
        // notes (work item / thread)
        commands::notes::add_work_note,
        commands::notes::add_thread_note,
        commands::notes::list_work_notes,
        commands::notes::list_thread_notes,
        commands::notes::delete_work_note,
        // wiki
        commands::wiki::list_wiki_notes,
        commands::wiki::get_wiki_note,
        commands::wiki::upsert_wiki_note,
        commands::wiki::delete_wiki_note,
        commands::wiki::search_wiki_titles,
        commands::wiki::search_wiki_bodies,
        // page visit
        commands::page_visit::record_page_visit,
        commands::page_visit::list_recent_page_visits,
        commands::page_visit::top_visited_pages,
        commands::page_visit::forget_page,
        commands::page_visit::count_page_visits_by_day,
        // usage
        commands::usage::record_usage,
        commands::usage::list_recent_usage,
        // code quality
        commands::code_quality::list_code_quality_scans,
        commands::code_quality::list_code_quality_findings,
        // snapshots
        commands::snapshot::list_snapshots,
        // branch
        commands::branch::list_branches,
        commands::branch::get_default_branch,
        commands::branch::rename_branch,
        commands::branch::delete_branch,
        commands::branch::list_local_branches,
        // git
        commands::git::get_repo_conflict_state,
        commands::git::get_ahead_behind,
        commands::git::append_to_gitignore,
        commands::git::restore_path,
        commands::git::git_fetch,
        commands::git::git_pull,
        commands::git::git_pull_remote_into_current,
        commands::git::git_push,
        commands::git::git_push_current_to,
        commands::git::git_merge_into,
        commands::git::git_rebase_onto,
        commands::git::git_commit_all,
        commands::git::git_add_path,
        commands::git::list_all_refs,
        commands::git::list_recent_remote_branches,
        commands::git::list_file_commits,
        commands::git::read_file_at_ref,
        commands::git::search_workspace_text,
        commands::git::list_existing_worktrees,
        commands::git::list_sibling_worktrees,
        commands::git::list_adoptable_worktrees,
        commands::git::git_blame,
        commands::git::get_branch_changes,
        // hooks / agent lifecycle
        commands::hooks::ingest_hook_event,
        commands::hooks::list_hook_events,
        commands::hooks::list_hook_events_by_kind,
        commands::hooks::list_agent_statuses,
        commands::hooks::list_open_agent_turns,
        commands::hooks::list_recent_agent_turns,
        // log
        commands::log::get_git_log,
        commands::log::get_commit_detail,
        commands::log::get_commits_ahead_of,
        // workspace
        commands::workspace::list_workspace_entries,
        commands::workspace::list_workspace_files,
        commands::workspace::read_workspace_file,
        commands::workspace::write_workspace_file,
        commands::workspace::create_workspace_file,
        commands::workspace::create_workspace_directory,
        commands::workspace::rename_workspace_path,
        commands::workspace::delete_workspace_path,
        commands::workspace::get_workspace_status_summary,
        // background tasks
        commands::background::list_background_tasks,
        commands::background::get_background_task,
        commands::background::start_background_task,
        commands::background::complete_background_task,
        commands::background::fail_background_task,
        commands::background::update_background_task,
        // followups
        commands::followup::list_followups,
        commands::followup::add_followup,
        commands::followup::remove_followup,
        commands::followup::clear_followups_for_thread,
        // webview
        commands::webview::open_external_url,
        commands::webview::clipboard_read_text,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the builder constructs without panicking.
    #[test]
    fn builder_constructs() {
        let _b = specta_builder();
    }

    /// Regenerate the TS bindings file the frontend imports.
    /// CI fails if `git diff` is non-empty after `cargo test`.
    #[test]
    fn export_ts_bindings() {
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
        let metadata = std::fs::metadata(&target).expect("bindings written");
        assert!(metadata.len() > 0, "bindings file should not be empty");
    }
}

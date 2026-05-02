//! Agent system-prompt assembly.
//!
//! Combines (in order):
//! 1. The repo-level `CLAUDE.md` (project instructions).
//! 2. A `<session-context>` block describing the active stream/thread.
//! 3. The thread's `custom_prompt` if set.
//! 4. The stream's `custom_prompt` if set.
//! 5. The user's `agentPromptAppend` from `oxplow.yaml`.
//!
//! The combined string is what oxplow passes via Claude's
//! `--append-system-prompt` flag (and/or the equivalent Copilot
//! mechanism).

use std::path::Path;

use oxplow_config::OxplowConfig;
use oxplow_domain::{Stream, Thread};

/// Read `<project>/CLAUDE.md` if it exists. Empty string otherwise.
pub fn load_claude_md(project_dir: &Path) -> String {
    let path = project_dir.join("CLAUDE.md");
    std::fs::read_to_string(&path).unwrap_or_default()
}

/// Build the `<session-context>` block. Includes worktree path,
/// stream/thread titles, and the thread's writer/read-only role.
pub fn build_session_context_block(stream: &Stream, thread: Option<&Thread>) -> String {
    let mut s = String::from("<session-context>\n");
    s.push_str(&format!("stream_id={}\n", stream.id));
    s.push_str(&format!("stream_title={}\n", stream.title));
    s.push_str(&format!("worktree_path={}\n", stream.worktree_path));
    s.push_str(&format!("branch={}\n", stream.branch));
    if let Some(t) = thread {
        s.push_str(&format!("thread_id={}\n", t.id));
        s.push_str(&format!("thread_title={}\n", t.title));
        s.push_str(&format!(
            "role={}\n",
            if t.status.is_writer() { "writer" } else { "read-only" }
        ));
    }
    s.push_str("</session-context>");
    s
}

/// Compose the full system-prompt suffix oxplow appends to whatever
/// the agent's built-in system prompt is. Sections are separated by
/// blank lines so Claude renders each as its own block.
pub fn assemble_system_prompt(
    project_dir: &Path,
    config: &OxplowConfig,
    stream: &Stream,
    thread: Option<&Thread>,
) -> String {
    let mut out = String::new();
    let claude_md = load_claude_md(project_dir);
    if !claude_md.is_empty() {
        out.push_str(&claude_md);
        out.push_str("\n\n");
    }
    out.push_str(&build_session_context_block(stream, thread));
    out.push_str("\n\n");
    if let Some(t) = thread {
        if let Some(prompt) = t.custom_prompt.as_deref().filter(|p| !p.is_empty()) {
            out.push_str(prompt);
            out.push_str("\n\n");
        }
    }
    if !config.agent_prompt_append.is_empty() {
        out.push_str(&config.agent_prompt_append);
        out.push_str("\n");
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::AgentKind;
    use oxplow_domain::{StreamId, StreamKind, ThreadId, ThreadStatus, Timestamp};
    use tempfile::tempdir;

    fn config() -> OxplowConfig {
        OxplowConfig {
            agent: AgentKind::Claude,
            project_name: "p".into(),
            lsp_servers: vec![],
            agent_prompt_append: "be precise".into(),
            snapshot_retention_days: 7,
            generated_dirs: vec![],
            snapshot_max_file_bytes: 1_000_000,
            inject_session_context: true,
        }
    }

    fn stream() -> Stream {
        Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "oxplow".into(),
            summary: String::new(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    fn thread() -> Thread {
        Thread {
            id: ThreadId::from("b-1"),
            stream_id: StreamId::from("s-1"),
            title: "explore".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: Some("Use TDD".into()),
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    #[test]
    fn session_context_includes_stream_and_thread_metadata() {
        let block = build_session_context_block(&stream(), Some(&thread()));
        assert!(block.contains("stream_id=s-1"));
        assert!(block.contains("thread_id=b-1"));
        assert!(block.contains("role=writer"));
    }

    #[test]
    fn assembled_prompt_concatenates_sections() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "## Repo rules\nRule 1.").unwrap();
        let prompt = assemble_system_prompt(dir.path(), &config(), &stream(), Some(&thread()));
        assert!(prompt.contains("Repo rules"));
        assert!(prompt.contains("<session-context>"));
        assert!(prompt.contains("Use TDD"));
        assert!(prompt.contains("be precise"));
    }

    #[test]
    fn missing_claude_md_is_silent() {
        let dir = tempdir().unwrap();
        let prompt = assemble_system_prompt(dir.path(), &config(), &stream(), Some(&thread()));
        // No "Repo rules" section but session-context still present.
        assert!(prompt.contains("<session-context>"));
    }

    #[test]
    fn read_only_thread_marks_role() {
        let mut t = thread();
        t.status = ThreadStatus::Queued;
        let block = build_session_context_block(&stream(), Some(&t));
        assert!(block.contains("role=read-only"));
    }
}

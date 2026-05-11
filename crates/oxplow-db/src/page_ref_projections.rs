//! Pure projections from per-kind data into [`PageRefEdge`]s.
//!
//! Each writer that owns a `source_kind` calls one of these helpers
//! to compute the edges its row contributes, then hands the result
//! to [`SqlitePageRefStore::replace_source`]. Keeping the
//! transformation pure (no DB, no IO) makes it trivially unit-
//! testable and lets the boot-time backfill replay the exact same
//! mapping from existing rows.
//!
//! Canonical id shapes (matching the frontend's `TabRef.id`):
//! - wiki:        `"<slug>"` for the wiki source kind
//! - task:        `"<integer id as string>"`
//! - file:        `"<repo-relative path>"`
//! - directory:   `"<repo-relative path, no trailing slash>"`
//! - finding:     `"<finding id>"`
//! - git-commit:  `"<sha>"`

use oxplow_domain::refs::{extract, RefVersion};
use oxplow_domain::{Task, TaskLink, TaskLinkType};

use crate::page_ref_store::PageRefEdge;

pub const KIND_WIKI: &str = "wiki";
pub const KIND_TASK: &str = "task";
pub const KIND_TASK_NOTE: &str = "task-note";
pub const KIND_FILE: &str = "file";
pub const KIND_DIRECTORY: &str = "directory";
pub const KIND_FINDING: &str = "finding";
pub const KIND_GIT_COMMIT: &str = "git-commit";

pub const RT_WIKI_FILE: &str = "wiki_file_ref";
pub const RT_WIKI_DIR: &str = "wiki_dir_ref";
pub const RT_WIKILINK: &str = "wikilink";
pub const RT_BODY_TASK: &str = "task_body_mention";
pub const RT_BODY_FINDING: &str = "finding_mention";
pub const RT_BODY_COMMIT: &str = "commit_mention";
pub const RT_TOUCHED_FILE: &str = "touched_file";
pub const RT_FINDING_PATH: &str = "finding_path";

/// Ref-types written by the task store from a body (title +
/// description + AC). Used by the slice-replace call so other
/// writers' rows for the same `task:<id>` source survive.
pub fn task_body_ref_types() -> Vec<String> {
    vec![
        RT_WIKI_FILE.to_string(),
        RT_WIKI_DIR.to_string(),
        RT_WIKILINK.to_string(),
        RT_BODY_TASK.to_string(),
        RT_BODY_FINDING.to_string(),
        RT_BODY_COMMIT.to_string(),
    ]
}

/// All link sub-types — the link store owns this slice for any
/// given source task.
pub fn task_link_ref_types() -> Vec<String> {
    [
        "blocks",
        "relates_to",
        "discovered_from",
        "duplicates",
        "supersedes",
        "replies_to",
    ]
    .iter()
    .map(|t| format!("task_link:{t}"))
    .collect()
}

/// Effort-touched-file slice owned by the effort store.
pub fn effort_ref_types() -> Vec<String> {
    vec![RT_TOUCHED_FILE.to_string()]
}

/// Edges contributed by a wiki page body. Owned by `wiki_pages` sync.
pub fn wiki_edges(slug: &str, body: &str) -> Vec<PageRefEdge> {
    let refs = extract(body);
    let mut out = Vec::new();
    for fd in refs.files_detail {
        let extra = match fd.version {
            RefVersion::Disk if fd.line.is_none() => None,
            _ => Some(
                serde_json::json!({
                    "line": fd.line,
                    "version": match &fd.version {
                        RefVersion::Disk => "disk".to_string(),
                        RefVersion::Ref(r) => format!("ref:{r}"),
                    }
                })
                .to_string(),
            ),
        };
        let mut edge = PageRefEdge::new(KIND_WIKI, slug, KIND_FILE, fd.path, RT_WIKI_FILE);
        if let Some(e) = extra {
            edge = edge.with_extra(e);
        }
        out.push(edge);
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_DIRECTORY,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(KIND_WIKI, slug, KIND_WIKI, w, RT_WIKILINK));
    }
    for t in refs.tasks {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_TASK,
            t.to_string(),
            RT_BODY_TASK,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_GIT_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Edges contributed by a task-note body. Single-owner source.
pub fn note_edges(note_id: &str, body: &str) -> Vec<PageRefEdge> {
    let refs = extract(body);
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_FILE,
            fd.path,
            RT_WIKI_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_DIRECTORY,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_WIKI,
            w,
            RT_WIKILINK,
        ));
    }
    for t in refs.tasks {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_TASK,
            t.to_string(),
            RT_BODY_TASK,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_GIT_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Edges contributed by a task's title + description + AC text.
pub fn task_edges(item: &Task) -> Vec<PageRefEdge> {
    let mut combined = String::new();
    combined.push_str(&item.title);
    combined.push('\n');
    combined.push_str(&item.description);
    if let Some(ac) = &item.acceptance_criteria {
        combined.push('\n');
        combined.push_str(ac);
    }
    let refs = extract(&combined);
    let id = item.id.to_string();
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            KIND_TASK,
            &id,
            KIND_FILE,
            fd.path,
            RT_WIKI_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_TASK,
            &id,
            KIND_DIRECTORY,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(KIND_TASK, &id, KIND_WIKI, w, RT_WIKILINK));
    }
    for t in refs.tasks {
        if t == item.id.value() {
            continue;
        }
        out.push(PageRefEdge::new(
            KIND_TASK,
            &id,
            KIND_TASK,
            t.to_string(),
            RT_BODY_TASK,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_TASK,
            &id,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_TASK,
            &id,
            KIND_GIT_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Touched-file edges for one effort.
pub fn effort_touched_file_edges(task_id: &str, paths: &[String]) -> Vec<PageRefEdge> {
    paths
        .iter()
        .map(|p| PageRefEdge::new(KIND_TASK, task_id, KIND_FILE, p.clone(), RT_TOUCHED_FILE))
        .collect()
}

fn link_type_str(t: TaskLinkType) -> &'static str {
    match t {
        TaskLinkType::Blocks => "blocks",
        TaskLinkType::RelatesTo => "relates_to",
        TaskLinkType::DiscoveredFrom => "discovered_from",
        TaskLinkType::Duplicates => "duplicates",
        TaskLinkType::Supersedes => "supersedes",
        TaskLinkType::RepliesTo => "replies_to",
    }
}

/// One edge per `TaskLink`. Source is `from_item`, target is
/// `to_item`, ref_type encodes the link sub-type.
pub fn link_edge(link: &TaskLink) -> PageRefEdge {
    PageRefEdge::new(
        KIND_TASK,
        link.from_item_id.to_string(),
        KIND_TASK,
        link.to_item_id.to_string(),
        format!("task_link:{}", link_type_str(link.link_type)),
    )
}

/// One edge per finding -> file. Owned by the findings writer.
pub fn finding_edges(finding_id: &str, path: &str) -> Vec<PageRefEdge> {
    vec![PageRefEdge::new(
        KIND_FINDING,
        finding_id,
        KIND_FILE,
        path,
        RT_FINDING_PATH,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{
        Task, TaskActorKind, TaskAuthor, TaskId, TaskLinkType, TaskPriority, TaskStatus, Timestamp,
    };

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn item(id: i64, title: &str, description: &str, ac: Option<&str>) -> Task {
        Task {
            id: TaskId::new(id),
            thread_id: None,
            parent_id: None,
            title: title.into(),
            description: description.into(),
            acceptance_criteria: ac.map(|s| s.to_string()),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: ts(),
            updated_at: ts(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
            category: None,
            tags: None,
        }
    }

    #[test]
    fn wiki_edges_cover_all_kinds() {
        let body = "[[src/app.rs]] [[dir:src]] [[architecture]] [[task:7]] [[finding:fnd-1]] [[git:abcdef0]]";
        let edges = wiki_edges("intro", body);
        let kinds: std::collections::BTreeSet<_> =
            edges.iter().map(|e| e.target_kind.as_str()).collect();
        assert!(kinds.contains("file"));
        assert!(kinds.contains("directory"));
        assert!(kinds.contains("wiki"));
        assert!(kinds.contains("task"));
        assert!(kinds.contains("finding"));
        assert!(kinds.contains("git-commit"));
    }

    #[test]
    fn task_edges_parse_ac_and_description() {
        let it = item(
            1,
            "fix something",
            "see [[src/app.rs]] for context, blocked by task:2",
            Some("touches finding:fnd-9"),
        );
        let edges = task_edges(&it);
        let targets: Vec<_> = edges
            .iter()
            .map(|e| (e.target_kind.as_str(), e.target_id.as_str()))
            .collect();
        assert!(targets.contains(&("file", "src/app.rs")));
        assert!(targets.contains(&("task", "2")));
        assert!(targets.contains(&("finding", "fnd-9")));
        // self-mention filtered out
        assert!(!targets.iter().any(|(k, id)| *k == "task" && *id == "1"));
    }

    #[test]
    fn effort_touched_file_edges_one_per_path() {
        let edges = effort_touched_file_edges("7", &["a.rs".into(), "b.rs".into()]);
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].source_id, "7");
        assert_eq!(edges[0].ref_type, "touched_file");
    }

    #[test]
    fn link_edge_labels_link_subtype() {
        use oxplow_domain::TaskLinkId;
        let link = TaskLink {
            id: TaskLinkId::new(1),
            thread_id: oxplow_domain::ThreadId::from("b-1"),
            from_item_id: TaskId::new(10),
            to_item_id: TaskId::new(20),
            link_type: TaskLinkType::Blocks,
            created_at: ts(),
        };
        let edge = link_edge(&link);
        assert_eq!(edge.source_id, "10");
        assert_eq!(edge.target_id, "20");
        assert_eq!(edge.ref_type, "task_link:blocks");
    }

    #[test]
    fn finding_edges_point_at_file() {
        let edges = finding_edges("fnd-7", "src/app.rs");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_kind, "finding");
        assert_eq!(edges[0].target_id, "src/app.rs");
    }
}

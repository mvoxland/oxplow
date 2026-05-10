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
//! - work-item:   `"wi-…"` (already prefixed)
//! - file:        `"<repo-relative path>"`
//! - directory:   `"<repo-relative path, no trailing slash>"`
//! - finding:     `"<finding id>"`
//! - git-commit:  `"<sha>"`

use oxplow_domain::refs::{extract, RefVersion};
use oxplow_domain::{WorkItem, WorkItemLink, WorkItemLinkType};

use crate::page_ref_store::PageRefEdge;

pub const KIND_WIKI: &str = "wiki";
pub const KIND_WORK_ITEM: &str = "work-item";
pub const KIND_WORK_NOTE: &str = "work-note";
pub const KIND_FILE: &str = "file";
pub const KIND_DIRECTORY: &str = "directory";
pub const KIND_FINDING: &str = "finding";
pub const KIND_GIT_COMMIT: &str = "git-commit";

pub const RT_WIKI_FILE: &str = "wiki_file_ref";
pub const RT_WIKI_DIR: &str = "wiki_dir_ref";
pub const RT_WIKILINK: &str = "wikilink";
pub const RT_BODY_WORK_ITEM: &str = "wi_body_mention";
pub const RT_BODY_FINDING: &str = "finding_mention";
pub const RT_BODY_COMMIT: &str = "commit_mention";
pub const RT_TOUCHED_FILE: &str = "touched_file";
pub const RT_FINDING_PATH: &str = "finding_path";

/// Ref-types written by the work-item store from a body (title +
/// description + AC). Used by the slice-replace call so other
/// writers' rows for the same `work-item:wi-X` source survive.
pub fn work_item_body_ref_types() -> Vec<String> {
    vec![
        RT_WIKI_FILE.to_string(),
        RT_WIKI_DIR.to_string(),
        RT_WIKILINK.to_string(),
        RT_BODY_WORK_ITEM.to_string(),
        RT_BODY_FINDING.to_string(),
        RT_BODY_COMMIT.to_string(),
    ]
}

/// All link sub-types — the link store owns this slice for any
/// given source work-item.
pub fn work_item_link_ref_types() -> Vec<String> {
    [
        "blocks",
        "relates_to",
        "discovered_from",
        "duplicates",
        "supersedes",
        "replies_to",
    ]
    .iter()
    .map(|t| format!("work_item_link:{t}"))
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
    for wi in refs.work_items {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_WORK_ITEM,
            wi,
            RT_BODY_WORK_ITEM,
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

/// Edges contributed by a work-note body (whether the note is
/// attached to a work-item or a thread). Single-owner source —
/// uses the full `replace_source` form.
pub fn note_edges(note_id: &str, body: &str) -> Vec<PageRefEdge> {
    let refs = extract(body);
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            KIND_WORK_NOTE,
            note_id,
            KIND_FILE,
            fd.path,
            RT_WIKI_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_WORK_NOTE,
            note_id,
            KIND_DIRECTORY,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            KIND_WORK_NOTE,
            note_id,
            KIND_WIKI,
            w,
            RT_WIKILINK,
        ));
    }
    for wi in refs.work_items {
        out.push(PageRefEdge::new(
            KIND_WORK_NOTE,
            note_id,
            KIND_WORK_ITEM,
            wi,
            RT_BODY_WORK_ITEM,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_WORK_NOTE,
            note_id,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_WORK_NOTE,
            note_id,
            KIND_GIT_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Edges contributed by a work item's title + description + AC text.
/// Owned by `work_item_store` upsert.
pub fn work_item_edges(item: &WorkItem) -> Vec<PageRefEdge> {
    let mut combined = String::new();
    combined.push_str(&item.title);
    combined.push('\n');
    combined.push_str(&item.description);
    if let Some(ac) = &item.acceptance_criteria {
        combined.push('\n');
        combined.push_str(ac);
    }
    let refs = extract(&combined);
    let id = item.id.as_str();
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            id,
            KIND_FILE,
            fd.path,
            RT_WIKI_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            id,
            KIND_DIRECTORY,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            id,
            KIND_WIKI,
            w,
            RT_WIKILINK,
        ));
    }
    for wi in refs.work_items {
        if wi == id {
            continue;
        }
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            id,
            KIND_WORK_ITEM,
            wi,
            RT_BODY_WORK_ITEM,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            id,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            id,
            KIND_GIT_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Touched-file edges for one effort. The work-item is the source so
/// "what files has this work-item touched" is one query. Multiple
/// efforts contribute their own slice via different `source_extra`s
/// — but for backlinks we collapse to a single edge per (wi, file).
pub fn effort_touched_file_edges(work_item_id: &str, paths: &[String]) -> Vec<PageRefEdge> {
    paths
        .iter()
        .map(|p| {
            PageRefEdge::new(
                KIND_WORK_ITEM,
                work_item_id,
                KIND_FILE,
                p.clone(),
                RT_TOUCHED_FILE,
            )
        })
        .collect()
}

fn link_type_str(t: WorkItemLinkType) -> &'static str {
    match t {
        WorkItemLinkType::Blocks => "blocks",
        WorkItemLinkType::RelatesTo => "relates_to",
        WorkItemLinkType::DiscoveredFrom => "discovered_from",
        WorkItemLinkType::Duplicates => "duplicates",
        WorkItemLinkType::Supersedes => "supersedes",
        WorkItemLinkType::RepliesTo => "replies_to",
    }
}

/// One edge per `WorkItemLink`. Source is `from_item`, target is
/// `to_item`, ref_type encodes the link sub-type so the renderer can
/// label it ("blocks", "relates to", …).
pub fn link_edge(link: &WorkItemLink) -> PageRefEdge {
    PageRefEdge::new(
        KIND_WORK_ITEM,
        link.from_item_id.as_str(),
        KIND_WORK_ITEM,
        link.to_item_id.as_str(),
        format!("work_item_link:{}", link_type_str(link.link_type)),
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
        Timestamp, WorkItem, WorkItemActorKind, WorkItemAuthor, WorkItemId, WorkItemKind,
        WorkItemLinkType, WorkItemPriority, WorkItemStatus,
    };

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn item(id: &str, title: &str, description: &str, ac: Option<&str>) -> WorkItem {
        WorkItem {
            id: WorkItemId::from(id),
            thread_id: None,
            parent_id: None,
            kind: WorkItemKind::Task,
            title: title.into(),
            description: description.into(),
            acceptance_criteria: ac.map(|s| s.to_string()),
            status: WorkItemStatus::Ready,
            priority: WorkItemPriority::Medium,
            sort_index: 0,
            created_by: WorkItemActorKind::User,
            created_at: ts(),
            updated_at: ts(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        }
    }

    #[test]
    fn wiki_edges_cover_all_kinds() {
        let body = "[[src/app.rs]] [[dir:src]] [[architecture]] [[wi-019abc-1]] [[finding:fnd-1]] [[git:abcdef0]]";
        let edges = wiki_edges("intro", body);
        let kinds: std::collections::BTreeSet<_> =
            edges.iter().map(|e| e.target_kind.as_str()).collect();
        assert!(kinds.contains("file"));
        assert!(kinds.contains("directory"));
        assert!(kinds.contains("wiki"));
        assert!(kinds.contains("work-item"));
        assert!(kinds.contains("finding"));
        assert!(kinds.contains("git-commit"));
    }

    #[test]
    fn work_item_edges_parse_ac_and_description() {
        let item = item(
            "wi-1",
            "fix something",
            "see [[src/app.rs]] for context, blocked by wi-019zzz-2",
            Some("touches finding:fnd-9"),
        );
        let edges = work_item_edges(&item);
        let targets: Vec<_> = edges
            .iter()
            .map(|e| (e.target_kind.as_str(), e.target_id.as_str()))
            .collect();
        assert!(targets.contains(&("file", "src/app.rs")));
        assert!(targets.contains(&("work-item", "wi-019zzz-2")));
        assert!(targets.contains(&("finding", "fnd-9")));
        // self-mention filtered out
        assert!(!targets.iter().any(|(_, id)| *id == "wi-1"));
    }

    #[test]
    fn effort_touched_file_edges_one_per_path() {
        let edges = effort_touched_file_edges("wi-7", &["a.rs".into(), "b.rs".into()]);
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].source_id, "wi-7");
        assert_eq!(edges[0].ref_type, "touched_file");
    }

    #[test]
    fn link_edge_labels_link_subtype() {
        let link = WorkItemLink {
            id: "wil-1".into(),
            thread_id: oxplow_domain::ThreadId::from("b-1"),
            from_item_id: WorkItemId::from("wi-a"),
            to_item_id: WorkItemId::from("wi-b"),
            link_type: WorkItemLinkType::Blocks,
            created_at: ts(),
        };
        let edge = link_edge(&link);
        assert_eq!(edge.source_id, "wi-a");
        assert_eq!(edge.target_id, "wi-b");
        assert_eq!(edge.ref_type, "work_item_link:blocks");
    }

    #[test]
    fn finding_edges_point_at_file() {
        let edges = finding_edges("fnd-7", "src/app.rs");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_kind, "finding");
        assert_eq!(edges[0].target_id, "src/app.rs");
    }
}

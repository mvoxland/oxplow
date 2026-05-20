//! Source-agnostic content-tree diff.
//!
//! A *content tree* is a `path -> content-id` map: the set of files at
//! one point in time, each tagged with an identity that changes iff the
//! file's content changes. [`diff_trees`] compares two such trees by
//! identity equality and reports per-path adds / modifications /
//! deletions.
//!
//! The identity is opaque to this module — it only ever compares for
//! equality. Both sides must therefore come from the **same source**:
//! a snapshot tree uses the xxh3-128 `blob_hash`, a git tree uses the
//! blob oid, and those identity spaces are not interchangeable. Keeping
//! the comparison here (and the identity production in each source) lets
//! snapshot-to-snapshot and commit-to-commit diffs share one
//! implementation instead of reaching for `git diff` or bespoke
//! per-caller logic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

/// How a path changed between two content trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ChangeStatus {
    /// Absent in `before`, present in `after`.
    Added,
    /// Present in both with a different content id.
    Modified,
    /// Present in `before`, absent in `after`.
    Deleted,
}

/// One changed path between two content trees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FileChange {
    pub path: String,
    pub status: ChangeStatus,
}

/// Diff two content trees (`path -> content-id`). Paths whose content
/// id is equal on both sides are omitted. Result is sorted by path.
///
/// Both maps must use the same identity space (see module docs).
pub fn diff_trees(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<FileChange> {
    let mut out: Vec<FileChange> = Vec::new();

    for (path, after_id) in after {
        match before.get(path) {
            None => out.push(FileChange {
                path: path.clone(),
                status: ChangeStatus::Added,
            }),
            Some(before_id) if before_id != after_id => out.push(FileChange {
                path: path.clone(),
                status: ChangeStatus::Modified,
            }),
            Some(_) => {} // unchanged
        }
    }
    for path in before.keys() {
        if !after.contains_key(path) {
            out.push(FileChange {
                path: path.clone(),
                status: ChangeStatus::Deleted,
            });
        }
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(p, h)| (p.to_string(), h.to_string()))
            .collect()
    }

    #[test]
    fn identical_trees_have_no_changes() {
        let t = tree(&[("a", "h1"), ("b", "h2")]);
        assert!(diff_trees(&t, &t).is_empty());
    }

    #[test]
    fn detects_added_modified_deleted() {
        let before = tree(&[("keep", "h"), ("mod", "h1"), ("gone", "h")]);
        let after = tree(&[("keep", "h"), ("mod", "h2"), ("new", "h")]);
        let changes = diff_trees(&before, &after);
        assert_eq!(
            changes,
            vec![
                FileChange {
                    path: "gone".into(),
                    status: ChangeStatus::Deleted
                },
                FileChange {
                    path: "mod".into(),
                    status: ChangeStatus::Modified
                },
                FileChange {
                    path: "new".into(),
                    status: ChangeStatus::Added
                },
            ]
        );
    }

    #[test]
    fn empty_before_is_all_added() {
        let after = tree(&[("a", "h"), ("b", "h")]);
        let changes = diff_trees(&BTreeMap::new(), &after);
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.status == ChangeStatus::Added));
    }

    #[test]
    fn empty_after_is_all_deleted() {
        let before = tree(&[("a", "h")]);
        let changes = diff_trees(&before, &BTreeMap::new());
        assert_eq!(
            changes,
            vec![FileChange {
                path: "a".into(),
                status: ChangeStatus::Deleted
            }]
        );
    }
}

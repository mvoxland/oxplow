//! Per-function churn: parse a unified diff and attribute added /
//! deleted line counts to the function each line lands in.
//!
//! The Change Analysis dashboard already knows file-level
//! `additions` / `deletions` and function-level
//! `complexityDelta` / `lengthDelta`, but not "function `foo` had
//! 8 lines added and 3 deleted." This module fills that gap.
//!
//! The output is best-effort: matching deletions on the base side
//! to the head-side function with the same qualified name (i.e.
//! `container::container::name`) handles the common case where a
//! function shifts position but keeps its identity. Functions that
//! were renamed or wholly removed surface only in the file-level
//! totals.
//!
//! `modified_lines` is `min(added, deleted)` per function — cheap,
//! explainable, and good enough as a "this function was edited
//! both ways" signal for ranking. Exact line-by-line alignment
//! would require Myers diff per function and isn't worth the cost.

use oxplow_code_metrics::FunctionMetrics;

/// Per-function churn for one file.
#[derive(Debug, Clone)]
pub struct FunctionChurn {
    pub name: String,
    pub container_path: Vec<String>,
    pub start_line_head: u32,
    pub added_lines: u32,
    pub deleted_lines: u32,
    pub modified_lines: u32,
}

/// File-level churn rollup with per-function breakdown.
#[derive(Debug, Clone)]
pub struct FileChurn {
    pub path: String,
    pub file_added: u32,
    pub file_deleted: u32,
    pub functions: Vec<FunctionChurn>,
}

/// Build per-function churn for one file.
///
/// `unified_diff` is the textual unified diff produced by
/// `git diff base -- path` (or equivalent). Empty / `None` callers
/// should short-circuit upstream — passing an empty string here
/// returns zeros.
///
/// `base_metrics` and `head_metrics` are the function lists already
/// produced by `analyze_file` on each side; we re-use them for
/// interval lookup instead of re-parsing.
pub fn compute_file_churn(
    path: &str,
    base_metrics: &[FunctionMetrics],
    head_metrics: &[FunctionMetrics],
    unified_diff: &str,
) -> FileChurn {
    let (added_in_head, deleted_in_base) = parse_unified_diff(unified_diff);
    let file_added = added_in_head.len() as u32;
    let file_deleted = deleted_in_base.len() as u32;

    // Empty diff or nothing parseable — keep file totals at zero
    // and return no per-function rows.
    if file_added == 0 && file_deleted == 0 {
        return FileChurn {
            path: path.to_string(),
            file_added,
            file_deleted,
            functions: Vec::new(),
        };
    }

    // Per-head-function counters keyed by qualified name. Using a
    // Vec keeps deterministic ordering and avoids pulling in a
    // hash dep just for this.
    let mut counters: Vec<(String, FunctionChurn)> = head_metrics
        .iter()
        .map(|m| {
            (
                qualified_key(&m.container_path, &m.name),
                FunctionChurn {
                    name: m.name.clone(),
                    container_path: m.container_path.clone(),
                    start_line_head: m.start_line,
                    added_lines: 0,
                    deleted_lines: 0,
                    modified_lines: 0,
                },
            )
        })
        .collect();

    // Sort head-function intervals by start_line for binary-search
    // attribution. Cheap; the file usually has only a few dozen
    // functions.
    let mut head_intervals: Vec<(u32, u32, usize)> = head_metrics
        .iter()
        .enumerate()
        .map(|(i, m)| (m.start_line, m.end_line, i))
        .collect();
    head_intervals.sort_by_key(|t| t.0);

    for line_no in added_in_head {
        if let Some(idx) = locate_interval(&head_intervals, line_no) {
            counters[idx].1.added_lines += 1;
        }
    }

    // Build head-side qkey lookup so we can map a base-side function
    // to the corresponding head row when attributing deletions.
    // Owned String keys so the immutable borrow on `counters` ends
    // before the deletion loop starts mutating them.
    let head_qkey_to_idx: std::collections::HashMap<String, usize> = counters
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.clone(), i))
        .collect();

    let mut base_intervals: Vec<(u32, u32, String)> = base_metrics
        .iter()
        .map(|m| {
            (
                m.start_line,
                m.end_line,
                qualified_key(&m.container_path, &m.name),
            )
        })
        .collect();
    base_intervals.sort_by_key(|t| t.0);

    for line_no in deleted_in_base {
        if let Some(qkey) = locate_qkey(&base_intervals, line_no) {
            if let Some(&idx) = head_qkey_to_idx.get(qkey) {
                counters[idx].1.deleted_lines += 1;
            }
            // Else: function existed in base but not in head (renamed
            // or removed). Counted at file level; not attributed to
            // any per-function row.
        }
    }

    let functions: Vec<FunctionChurn> = counters
        .into_iter()
        .map(|(_, mut c)| {
            c.modified_lines = c.added_lines.min(c.deleted_lines);
            c
        })
        .filter(|c| c.added_lines > 0 || c.deleted_lines > 0)
        .collect();

    FileChurn {
        path: path.to_string(),
        file_added,
        file_deleted,
        functions,
    }
}

fn qualified_key(container_path: &[String], name: &str) -> String {
    if container_path.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", container_path.join("::"), name)
    }
}

/// Walk `intervals` (sorted by start_line) and return the index of
/// the first one whose `[start, end]` range contains `line`.
/// Functions can nest in some grammars; we attribute to the
/// outermost match for stability — caller iterates lines, so the
/// first sorted match is the outermost containing function.
fn locate_interval(intervals: &[(u32, u32, usize)], line: u32) -> Option<usize> {
    for &(start, end, idx) in intervals {
        if start <= line && line <= end {
            return Some(idx);
        }
        if start > line {
            break;
        }
    }
    None
}

fn locate_qkey<'a>(intervals: &'a [(u32, u32, String)], line: u32) -> Option<&'a str> {
    for (start, end, key) in intervals {
        if *start <= line && line <= *end {
            return Some(key.as_str());
        }
        if *start > line {
            break;
        }
    }
    None
}

/// Parse a unified diff and return `(added_lines_in_new, deleted_lines_in_old)`
/// — vectors of 1-indexed line numbers in their respective sides.
///
/// Tolerant: skips header lines (`---`, `+++`, `diff --git`,
/// `index`, `\\ No newline at end of file`); re-syncs on each `@@`
/// hunk header. Lines outside any hunk are ignored.
fn parse_unified_diff(diff: &str) -> (Vec<u32>, Vec<u32>) {
    let mut added: Vec<u32> = Vec::new();
    let mut deleted: Vec<u32> = Vec::new();

    let mut old_lineno: u32 = 0;
    let mut new_lineno: u32 = 0;
    let mut in_hunk = false;

    for raw in diff.lines() {
        if let Some(rest) = raw.strip_prefix("@@") {
            // Form: `@@ -old_start[,old_count] +new_start[,new_count] @@ ...`
            let Some((header, _)) = rest.split_once("@@") else {
                in_hunk = false;
                continue;
            };
            let parts: Vec<&str> = header.split_whitespace().collect();
            if parts.len() < 2 {
                in_hunk = false;
                continue;
            }
            let old_start = parts
                .iter()
                .find(|p| p.starts_with('-'))
                .and_then(parse_hunk_start);
            let new_start = parts
                .iter()
                .find(|p| p.starts_with('+'))
                .and_then(parse_hunk_start);
            match (old_start, new_start) {
                (Some(o), Some(n)) => {
                    old_lineno = o;
                    new_lineno = n;
                    in_hunk = true;
                }
                _ => {
                    in_hunk = false;
                }
            }
            continue;
        }
        if !in_hunk {
            continue;
        }
        if raw.starts_with("\\ ") {
            // "\ No newline at end of file" — bookkeeping noise.
            continue;
        }
        if let Some(_body) = raw.strip_prefix('+') {
            if raw.starts_with("+++") {
                continue;
            }
            added.push(new_lineno);
            new_lineno += 1;
        } else if let Some(_body) = raw.strip_prefix('-') {
            if raw.starts_with("---") {
                continue;
            }
            deleted.push(old_lineno);
            old_lineno += 1;
        } else if raw.starts_with(' ') || raw.is_empty() {
            // Context line — present on both sides.
            old_lineno += 1;
            new_lineno += 1;
        } else {
            // Unrecognized — probably a section delimiter from a
            // combined diff. Bail out of the hunk to avoid drift.
            in_hunk = false;
        }
    }
    (added, deleted)
}

fn parse_hunk_start(p: &&str) -> Option<u32> {
    // Strip the leading '-' or '+' then take whatever's before any
    // ',count' marker.
    let s = p.trim_start_matches(['-', '+']);
    let (start, _count) = s.split_once(',').unwrap_or((s, ""));
    start.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_code_metrics::Visibility;

    fn fm(name: &str, start: u32, end: u32) -> FunctionMetrics {
        FunctionMetrics {
            path: "src/lib.rs".into(),
            name: name.into(),
            complexity: 1,
            length: end - start + 1,
            parameter_count: 0,
            start_line: start,
            end_line: end,
            container_path: Vec::new(),
            visibility: Visibility::Unknown,
        }
    }

    #[test]
    fn parses_simple_hunk() {
        let diff = "\
@@ -10,3 +10,4 @@
 context
-old
+new1
+new2
 context
";
        let (added, deleted) = parse_unified_diff(diff);
        assert_eq!(added, vec![11, 12]);
        assert_eq!(deleted, vec![11]);
    }

    #[test]
    fn skips_diff_headers() {
        let diff = "\
diff --git a/x b/x
index abc..def 100644
--- a/x
+++ b/x
@@ -1,2 +1,3 @@
 a
+b
 c
";
        let (added, deleted) = parse_unified_diff(diff);
        assert_eq!(added, vec![2]);
        assert!(deleted.is_empty());
    }

    #[test]
    fn attributes_added_lines_to_containing_head_function() {
        let head = vec![fm("foo", 5, 20), fm("bar", 25, 40)];
        let base = vec![fm("foo", 5, 18), fm("bar", 23, 38)];
        // Add lines 19, 20 inside foo and 26, 27 inside bar.
        let diff = "\
@@ -18,2 +18,4 @@
 line18
+added19
+added20
 line19
@@ -25,2 +27,3 @@
 line25
+added26
 line26
";
        let churn = compute_file_churn("src/lib.rs", &base, &head, diff);
        assert_eq!(churn.file_added, 3);
        let foo = churn.functions.iter().find(|f| f.name == "foo").unwrap();
        let bar = churn.functions.iter().find(|f| f.name == "bar").unwrap();
        assert_eq!(foo.added_lines, 2);
        assert_eq!(bar.added_lines, 1);
    }

    #[test]
    fn deletions_attribute_via_qualified_key_to_head_side() {
        // `foo` exists on both sides at different line ranges; a
        // deletion in base inside `foo`'s base range should bump
        // `foo`'s deleted count on the head row.
        let head = vec![fm("foo", 5, 8)];
        let base = vec![fm("foo", 5, 12)];
        let diff = "\
@@ -8,4 +8,1 @@
 line8
-line9
-line10
-line11
";
        let churn = compute_file_churn("src/lib.rs", &base, &head, diff);
        let foo = churn.functions.iter().find(|f| f.name == "foo").unwrap();
        assert_eq!(foo.deleted_lines, 3);
        assert_eq!(churn.file_deleted, 3);
    }

    #[test]
    fn modified_lines_is_min_of_added_and_deleted() {
        let head = vec![fm("foo", 1, 10)];
        let base = vec![fm("foo", 1, 9)];
        let diff = "\
@@ -1,9 +1,10 @@
-old1
-old2
-old3
+new1
+new2
+new3
+new4
 ctx
 ctx
 ctx
 ctx
 ctx
 ctx
";
        let churn = compute_file_churn("src/lib.rs", &base, &head, diff);
        let foo = &churn.functions[0];
        assert_eq!(foo.added_lines, 4);
        assert_eq!(foo.deleted_lines, 3);
        assert_eq!(foo.modified_lines, 3);
    }

    #[test]
    fn deletion_in_removed_function_falls_to_file_level_only() {
        // Function `gone` exists in base but not in head — its
        // deleted lines count toward file_deleted but no per-function
        // row is produced for it on the head side.
        let head: Vec<FunctionMetrics> = Vec::new();
        let base = vec![fm("gone", 1, 5)];
        let diff = "\
@@ -1,5 +0,0 @@
-line1
-line2
-line3
-line4
-line5
";
        let churn = compute_file_churn("src/x.rs", &base, &head, diff);
        assert_eq!(churn.file_deleted, 5);
        assert!(churn.functions.is_empty());
    }

    #[test]
    fn empty_diff_produces_zero_churn() {
        let churn = compute_file_churn("src/x.rs", &[], &[], "");
        assert_eq!(churn.file_added, 0);
        assert_eq!(churn.file_deleted, 0);
        assert!(churn.functions.is_empty());
    }
}

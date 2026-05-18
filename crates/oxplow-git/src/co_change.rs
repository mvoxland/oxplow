//! Co-change history analysis: which files are usually committed
//! together?
//!
//! Walks the commit graph, looks at the set of paths touched by
//! each commit, and accumulates two maps:
//!
//! - **co-change counts**: `(file_a, file_b) → how often they
//!   appear in the same commit`.
//! - **last-touched timestamp**: `file → most recent commit
//!   seconds-since-epoch`.
//!
//! Given those two maps and the file set of the diff under review,
//! [`analyze_surprise`] classifies each file as either:
//!
//! - `Normal` — touched alongside its usual co-changers, or has no
//!   strong prior co-changers.
//! - `UsualCoChangersAbsent` — historically this file moves with X
//!   and Y, but X and Y aren't in this commit.
//! - `Dormant { last_touched_days }` — file hasn't been touched in
//!   a long time.
//!
//! This is the behavioural answer to "weird that this was touched"
//! — it's the CodeScene `change_coupling` signal restated.
//!
//! Pure, no I/O once `build_history` returns. Caller can cache the
//! [`CoChangeHistory`] per `(repo, window_days)` and re-query it
//! cheaply for every diff.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

/// How many days back to walk by default (~6 months). Picked to
/// span at least a couple of release cycles for most projects.
pub const DEFAULT_WINDOW_DAYS: i64 = 180;

/// Minimum number of co-occurrences before a file is considered to
/// have a "usual co-changer." Smaller than this and the signal is
/// noise — every file co-occurs with the README once.
pub const DEFAULT_MIN_COOCCURRENCES: u32 = 3;

/// A file is "dormant" when it hasn't been touched within this
/// window of days (90 = roughly a quarter).
pub const DEFAULT_DORMANT_DAYS: i64 = 90;

/// Skip absurdly large commits (mass renames, mass formatter runs)
/// when accumulating co-change. They dominate the signal otherwise.
pub const COMMIT_FILE_LIMIT: usize = 50;

/// Build options.
#[derive(Debug, Clone)]
pub struct CoChangeOptions {
    /// Calendar-time window. Commits older than this are ignored.
    pub window_days: i64,
    /// Maximum number of commits to walk (safety cap; default 5000).
    pub commit_cap: usize,
    /// Skip commits that touch more than this many files (heuristic
    /// to ignore mass renames).
    pub commit_file_limit: usize,
}

impl Default for CoChangeOptions {
    fn default() -> Self {
        Self {
            window_days: DEFAULT_WINDOW_DAYS,
            commit_cap: 5000,
            commit_file_limit: COMMIT_FILE_LIMIT,
        }
    }
}

/// Pre-aggregated co-change history for a repo. Built once, queried
/// many times.
#[derive(Debug, Clone)]
pub struct CoChangeHistory {
    /// `file → frequent_co_changers` (sorted by descending count).
    /// Only files with ≥ `DEFAULT_MIN_COOCCURRENCES` are listed.
    co_changers: HashMap<String, Vec<(String, u32)>>,
    /// `file → most recent touch timestamp (seconds since epoch)`.
    last_touched: HashMap<String, i64>,
    /// Time the analysis was performed (seconds since epoch). Used
    /// to compute dormancy in days at query time.
    analyzed_at_secs: i64,
}

impl CoChangeHistory {
    /// Empty history — used as a fallback when the repo can't be
    /// opened (every query returns `Normal`).
    pub fn empty() -> Self {
        Self {
            co_changers: HashMap::new(),
            last_touched: HashMap::new(),
            analyzed_at_secs: now_secs(),
        }
    }

    /// Total number of files with at least one recorded touch.
    pub fn file_count(&self) -> usize {
        self.last_touched.len()
    }

    /// Up to N most frequent co-changers for `file`.
    pub fn co_changers_for(&self, file: &str, max: usize) -> &[(String, u32)] {
        match self.co_changers.get(file) {
            Some(v) => &v[..v.len().min(max)],
            None => &[],
        }
    }

    /// Most-recent touch timestamp for `file`, if any.
    pub fn last_touched_secs(&self, file: &str) -> Option<i64> {
        self.last_touched.get(file).copied()
    }
}

/// Why a file was flagged as surprising.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurpriseReason {
    /// Nothing surprising — file has no strong co-changers, OR its
    /// usual co-changers are also in this commit.
    Normal,
    /// The file has well-established co-changers (≥ N co-occurrences
    /// historically), but none of them are in this commit. Carries
    /// the top-3 expected co-changers for the tooltip.
    UsualCoChangersAbsent { expected: Vec<String> },
    /// File hasn't been touched in `last_touched_days`. Threshold
    /// is `DEFAULT_DORMANT_DAYS` unless the caller overrode it.
    Dormant { last_touched_days: i64 },
}

impl SurpriseReason {
    pub fn is_surprising(&self) -> bool {
        !matches!(self, SurpriseReason::Normal)
    }
}

/// One row of [`analyze_surprise`] output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FileSurprise {
    pub path: String,
    pub reason: SurpriseReason,
}

/// Walk the repo's commit graph and build a [`CoChangeHistory`].
///
/// On error (repo not found, walk fails, etc.) returns an empty
/// history rather than panicking — the caller can use it as a
/// no-op (`analyze_surprise` will return all-Normal).
pub fn build_history(repo_path: &Path, options: CoChangeOptions) -> CoChangeHistory {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return CoChangeHistory::empty();
    };
    let Ok(mut walk) = repo.revwalk() else {
        return CoChangeHistory::empty();
    };
    if walk.push_head().is_err() {
        return CoChangeHistory::empty();
    }
    let _ = walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL);

    let now = now_secs();
    let cutoff = now - options.window_days * 86_400;

    // pair_counts is (file_a, file_b) → count, with a < b enforced
    // so each pair is counted once and lookups are symmetric.
    let mut pair_counts: HashMap<(String, String), u32> = HashMap::new();
    let mut last_touched: HashMap<String, i64> = HashMap::new();
    let mut commit_count = 0usize;

    for oid in walk.flatten() {
        if commit_count >= options.commit_cap {
            break;
        }
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let ts = commit.time().seconds();
        if ts < cutoff {
            // Sort order is time-descending; once we cross the
            // cutoff every remaining commit is older too.
            break;
        }
        let files = commit_files(&repo, &commit);
        if files.is_empty() || files.len() > options.commit_file_limit {
            // Skip empties (merge commits with no diff) and
            // mega-commits (mass renames / formatter sweeps) — both
            // would drown the signal.
            continue;
        }
        for f in &files {
            let entry = last_touched.entry(f.clone()).or_insert(ts);
            if ts > *entry {
                *entry = ts;
            }
        }
        // Increment every pair count.
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let (a, b) = if files[i] < files[j] {
                    (files[i].clone(), files[j].clone())
                } else {
                    (files[j].clone(), files[i].clone())
                };
                *pair_counts.entry((a, b)).or_insert(0) += 1;
            }
        }
        commit_count += 1;
    }

    // Flatten pair_counts into per-file sorted lists, filtered to
    // pairs with at least DEFAULT_MIN_COOCCURRENCES.
    let mut co_changers: HashMap<String, Vec<(String, u32)>> = HashMap::new();
    for ((a, b), count) in pair_counts {
        if count < DEFAULT_MIN_COOCCURRENCES {
            continue;
        }
        co_changers
            .entry(a.clone())
            .or_default()
            .push((b.clone(), count));
        co_changers.entry(b).or_default().push((a, count));
    }
    for list in co_changers.values_mut() {
        list.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
    }

    CoChangeHistory {
        co_changers,
        last_touched,
        analyzed_at_secs: now,
    }
}

/// Classify every file in `commit_files` against the history.
///
/// `commit_files` is the set of paths touched by the diff the user
/// is reviewing. Order is preserved in the output.
pub fn analyze_surprise(
    history: &CoChangeHistory,
    commit_files: &[String],
    dormant_days: i64,
) -> Vec<FileSurprise> {
    let touched: std::collections::HashSet<&str> =
        commit_files.iter().map(|s| s.as_str()).collect();

    commit_files
        .iter()
        .map(|file| {
            // 1. Dormancy check first — a long-dormant file is the
            //    cheaper, clearer signal and shouldn't be masked by
            //    the co-changer check.
            if let Some(ts) = history.last_touched_secs(file) {
                let days = (history.analyzed_at_secs - ts) / 86_400;
                if days >= dormant_days {
                    return FileSurprise {
                        path: file.clone(),
                        reason: SurpriseReason::Dormant {
                            last_touched_days: days,
                        },
                    };
                }
            } else {
                // File never touched in the window — treat as
                // dormant with the window itself as the floor.
                return FileSurprise {
                    path: file.clone(),
                    reason: SurpriseReason::Dormant {
                        last_touched_days: dormant_days.max(1),
                    },
                };
            }

            // 2. Co-changer check.
            let usual = history.co_changers_for(file, 3);
            if usual.is_empty() {
                return FileSurprise {
                    path: file.clone(),
                    reason: SurpriseReason::Normal,
                };
            }
            let any_present = usual
                .iter()
                .any(|(other, _)| touched.contains(other.as_str()));
            if any_present {
                FileSurprise {
                    path: file.clone(),
                    reason: SurpriseReason::Normal,
                }
            } else {
                FileSurprise {
                    path: file.clone(),
                    reason: SurpriseReason::UsualCoChangersAbsent {
                        expected: usual.iter().map(|(o, _)| o.clone()).collect(),
                    },
                }
            }
        })
        .collect()
}

/// Collect the file paths touched by a commit. For root commits
/// (no parent) we diff against an empty tree, so initial commits
/// contribute their full content.
fn commit_files(repo: &git2::Repository, commit: &git2::Commit) -> Vec<String> {
    let Ok(tree) = commit.tree() else {
        return Vec::new();
    };
    // Diff against the first parent (if any). For merge commits
    // this misses some changes — acceptable: we're looking for
    // co-change signal, not perfect file attribution.
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .ok();
    let Some(diff) = diff else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let _ = diff.foreach(
        &mut |delta, _| {
            // Prefer new_file path; fall back to old_file for deletes.
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned());
            if let Some(p) = path {
                out.push(p);
            }
            true
        },
        None,
        None,
        None,
    );
    // Deduplicate (a path can appear in both old and new for a
    // single delta).
    out.sort();
    out.dedup();
    out
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// Run a `git` command in the given dir. Silences stdout/stderr
    /// so the test output stays readable.
    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .expect("git failed");
        assert!(output.status.success(), "git {:?} failed", args);
    }

    fn write_and_commit(dir: &Path, files: &[(&str, &str)], message: &str) {
        for (p, contents) in files {
            let full = dir.join(p);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, contents).unwrap();
            git(dir, &["add", p]);
        }
        git(dir, &["commit", "-m", message]);
    }

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        dir
    }

    #[test]
    fn empty_repo_yields_empty_history() {
        let dir = init_repo();
        let history = build_history(dir.path(), CoChangeOptions::default());
        assert_eq!(history.file_count(), 0);
        let result = analyze_surprise(&history, &["any.rs".into()], DEFAULT_DORMANT_DAYS);
        // No record → file is "dormant" (we treat absent === unknown
        // === surprising-by-default).
        assert!(matches!(result[0].reason, SurpriseReason::Dormant { .. }));
    }

    #[test]
    fn co_changers_detected_after_enough_history() {
        let dir = init_repo();
        // a.rs + b.rs change together 4 times; c.rs is alone.
        for i in 0..4 {
            write_and_commit(
                dir.path(),
                &[("a.rs", &format!("a-v{i}")), ("b.rs", &format!("b-v{i}"))],
                &format!("ab pair {i}"),
            );
        }
        write_and_commit(dir.path(), &[("c.rs", "c-v0")], "c alone");

        let history = build_history(dir.path(), CoChangeOptions::default());
        let co_a = history.co_changers_for("a.rs", 3);
        assert_eq!(co_a.len(), 1);
        assert_eq!(co_a[0].0, "b.rs");
        assert_eq!(co_a[0].1, 4);

        // Touching a.rs WITH b.rs is normal.
        let normal = analyze_surprise(
            &history,
            &["a.rs".into(), "b.rs".into()],
            DEFAULT_DORMANT_DAYS,
        );
        assert!(matches!(normal[0].reason, SurpriseReason::Normal));
        assert!(matches!(normal[1].reason, SurpriseReason::Normal));

        // Touching a.rs WITHOUT b.rs is surprising.
        let lonely = analyze_surprise(
            &history,
            &["a.rs".into(), "unrelated.rs".into()],
            DEFAULT_DORMANT_DAYS,
        );
        match &lonely[0].reason {
            SurpriseReason::UsualCoChangersAbsent { expected } => {
                assert_eq!(expected, &vec!["b.rs".to_string()]);
            }
            other => panic!("expected UsualCoChangersAbsent, got {other:?}"),
        }
    }

    #[test]
    fn dormancy_overrides_co_changers() {
        // With dormant_days=0 every previously-touched file is
        // "dormant" — this is the cheap way to assert that the
        // dormancy branch fires (without faking timestamps).
        let dir = init_repo();
        for i in 0..3 {
            write_and_commit(
                dir.path(),
                &[("a.rs", &format!("a{i}")), ("b.rs", &format!("b{i}"))],
                &format!("c{i}"),
            );
        }
        let history = build_history(dir.path(), CoChangeOptions::default());
        let result = analyze_surprise(&history, &["a.rs".into()], 0);
        match &result[0].reason {
            SurpriseReason::Dormant { last_touched_days } => {
                assert!(*last_touched_days >= 0);
            }
            other => panic!("expected Dormant, got {other:?}"),
        }
    }

    #[test]
    fn unknown_file_is_treated_as_dormant() {
        let dir = init_repo();
        write_and_commit(dir.path(), &[("a.rs", "x")], "init");
        let history = build_history(dir.path(), CoChangeOptions::default());
        let result = analyze_surprise(&history, &["never-existed.rs".into()], DEFAULT_DORMANT_DAYS);
        assert!(matches!(result[0].reason, SurpriseReason::Dormant { .. }));
    }

    #[test]
    fn huge_commits_are_skipped() {
        let dir = init_repo();
        // Build a "mass rename" commit with > COMMIT_FILE_LIMIT files.
        let mut files = Vec::new();
        let mut owned: Vec<(String, String)> = Vec::new();
        for i in 0..(COMMIT_FILE_LIMIT + 5) {
            owned.push((format!("f{i}.rs"), format!("v{i}")));
        }
        for (p, c) in &owned {
            files.push((p.as_str(), c.as_str()));
        }
        write_and_commit(dir.path(), &files, "mass");

        let history = build_history(dir.path(), CoChangeOptions::default());
        // The mass commit shouldn't have contributed pair counts —
        // every file should have zero co-changers.
        for (p, _) in &owned {
            assert!(history.co_changers_for(p, 3).is_empty());
        }
    }
}

//! Code-quality scanners.
//!
//! Two analysis pipelines, both running in-process:
//!
//! - [`run_metrics_scan`] — per-function metrics (complexity, length,
//!   parameter count) via `oxplow-code-metrics`.
//! - [`run_duplication_scan`] — token-stream duplicate-block detection
//!   via `oxplow-code-dup`.
//!
//! The store + IPC contract refer to these by the analysis-kind names
//! `"metrics"` and `"duplication"`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::collections::BTreeSet;

use oxplow_code_dup::{detect_duplicates, detect_duplicates_scoped, DupOptions};
use oxplow_code_metrics::FunctionMetrics;
use oxplow_tree_source::{
    collect_corpus, AllFiles, DiskTreeSource, FileFilter, TreeError, TreeSource,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum CodeQualityError {
    /// Surfaces a failure inside the spawn_blocking pool (panic or
    /// joining error).
    #[error("scan task failed: {0}")]
    Task(String),
    /// The scan exceeded the configured wall-clock budget.
    #[error("scan timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// Tree source enumeration / read failed (git error, IO error,
    /// snapshot stub).
    #[error("tree source failed: {0}")]
    TreeSource(String),
}

impl From<TreeError> for CodeQualityError {
    fn from(e: TreeError) -> Self {
        CodeQualityError::TreeSource(format!("{e}"))
    }
}

/// Default wall-clock budget for a single scan. Tunable via
/// `RunOptions::timeout`.
const DEFAULT_SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// One finding the renderer surfaces in the code-quality panel.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CodeQualityFinding {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    /// e.g. `"complexity"`, `"function-length"`, `"parameter-count"`,
    /// `"duplicate-block"`.
    pub kind: String,
    pub metric_value: f64,
    /// Free-form JSON for analysis-specific metadata. The store
    /// persists this as a string column.
    pub extra_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Subset of repo-relative paths. Empty = scan whole repo.
    pub files: Vec<String>,
    /// Wall-clock budget. `None` uses [`DEFAULT_SCAN_TIMEOUT`].
    pub timeout: Option<std::time::Duration>,
}

/// Build the file list to analyze: either the explicit list, or every
/// supported file under `project_dir`. Skips dotdirs (`.git`,
/// `.cargo`, …) and the usual build/dep folders.
fn collect_supported_files(project_dir: &Path, opts: &RunOptions) -> Vec<PathBuf> {
    if !opts.files.is_empty() {
        return opts
            .files
            .iter()
            .map(|p| project_dir.join(p))
            .filter(|p| oxplow_code_metrics::is_supported_path(p))
            .collect();
    }
    let skip = ["target", "node_modules", "dist", "build", ".git"];
    WalkDir::new(project_dir)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if e.depth() == 0 {
                return true;
            }
            if name.starts_with('.') && e.file_type().is_dir() {
                return false;
            }
            !(e.file_type().is_dir() && skip.contains(&name.as_ref()))
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| oxplow_code_metrics::is_supported_path(p))
        .collect()
}

fn metrics_to_findings(
    project_dir: &Path,
    metrics: Vec<FunctionMetrics>,
) -> Vec<CodeQualityFinding> {
    let mut out = Vec::with_capacity(metrics.len() * 3);
    for m in metrics {
        // m.path may be absolute (we built it from `project_dir.join`).
        // Re-derive the repo-relative form so the panel matches the
        // path strings the rest of the system speaks.
        let path = match Path::new(&m.path).strip_prefix(project_dir) {
            Ok(rel) => rel.to_string_lossy().to_string(),
            Err(_) => m.path.clone(),
        };
        let extra = format!(
            r#"{{"functionName":{}}}"#,
            serde_json::to_string(&m.name).unwrap_or_else(|_| "\"\"".into())
        );
        // Complexity is always >=1 in our model (decision points + 1)
        // and length is always >=1 (end_line - start_line + 1), so both
        // findings unconditionally emit. Only parameter-count is gated.
        out.push(CodeQualityFinding {
            path: path.clone(),
            start_line: m.start_line,
            end_line: m.end_line,
            kind: "complexity".into(),
            metric_value: m.complexity as f64,
            extra_json: Some(extra.clone()),
        });
        out.push(CodeQualityFinding {
            path: path.clone(),
            start_line: m.start_line,
            end_line: m.end_line,
            kind: "function-length".into(),
            metric_value: m.length as f64,
            extra_json: Some(extra.clone()),
        });
        if m.parameter_count > 0 {
            out.push(CodeQualityFinding {
                path,
                start_line: m.start_line,
                end_line: m.end_line,
                kind: "parameter-count".into(),
                metric_value: m.parameter_count as f64,
                extra_json: Some(extra),
            });
        }
    }
    out
}

/// Per-function metrics scan: walks every supported file under
/// `project_dir` (or the explicit file list), computes complexity /
/// length / parameter count via tree-sitter, and fans the records
/// out to one [`CodeQualityFinding`] per metric.
pub async fn run_metrics_scan(
    project_dir: &Path,
    opts: RunOptions,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let project = project_dir.to_path_buf();
    let timeout = opts.timeout.unwrap_or(DEFAULT_SCAN_TIMEOUT);
    // The metric pass is CPU-bound; punt to a blocking pool so we
    // don't stall the tokio runtime on large repos.
    let task = tokio::task::spawn_blocking(move || -> Result<_, CodeQualityError> {
        let files = collect_supported_files(&project, &opts);
        let mut metrics = Vec::new();
        for path in files {
            let source = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue, // unreadable / binary — skip
            };
            metrics.extend(oxplow_code_metrics::analyze_file(
                &path.to_string_lossy(),
                &source,
            ));
        }
        Ok(metrics_to_findings(&project, metrics))
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(inner)) => inner,
        Ok(Err(join_err)) => Err(CodeQualityError::Task(format!("metrics task: {join_err}"))),
        Err(_) => Err(CodeQualityError::Timeout(timeout)),
    }
}

/// Duplicate-block scan against an arbitrary tree version.
///
/// `source` enumerates files and reads their content (Disk = working
/// tree, GitRef = a commit's tree, …); `filter` decides which paths
/// from the source make it into the corpus. Every pair of corpus
/// docs is matched, including same-file self-matches — this is the
/// "scan everything" mode used by the standalone Code Quality panel.
/// The change-analysis flow uses
/// [`run_duplication_scan_scoped`] instead so unchanged peers
/// participate as match targets without adding their own findings.
///
/// The whole scan runs on `spawn_blocking` because tree-sitter and
/// libgit2 are CPU/IO-bound; the trait objects are `Send + Sync` so
/// `Arc`-wrapping them lets us move references into the blocking
/// pool.
pub async fn run_duplication_scan_with(
    source: Arc<dyn TreeSource>,
    filter: Arc<dyn FileFilter>,
    timeout: Option<std::time::Duration>,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let timeout = timeout.unwrap_or(DEFAULT_SCAN_TIMEOUT);
    let task = tokio::task::spawn_blocking(move || -> Result<_, CodeQualityError> {
        let corpus = collect_corpus(source.as_ref(), filter.as_ref())?;
        // Drop entries the metrics layer can't parse — the detector
        // tolerates unsupported files but we'd rather not feed them
        // through tree-sitter at all.
        let inputs: Vec<(String, String)> = corpus
            .into_iter()
            .filter(|(p, _)| oxplow_code_metrics::is_supported_path(Path::new(p)))
            .collect();
        let blocks = detect_duplicates(inputs, DupOptions::default());
        Ok(blocks_to_findings(blocks))
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(inner)) => inner,
        Ok(Err(join_err)) => Err(CodeQualityError::Task(format!("duplication task: {join_err}"))),
        Err(_) => Err(CodeQualityError::Timeout(timeout)),
    }
}

/// Scoped duplicate-block scan: corpus is the WHOLE tree (every
/// supported file the source enumerates), but a finding only
/// surfaces when at least one side's path is in `scope_filter`. The
/// scope-side is rotated to side A so the renderer's
/// "you're analyzing this file vs the peer over there" convention
/// holds. Same-path matches (a region of a file matching another
/// region of the SAME file) are dropped — those are almost always
/// shifted-by-one winnowing artifacts on long token streams.
///
/// This is what the change-analysis page wants: when a user changes
/// `foo.ts`, surface duplications between `foo.ts` and ANY existing
/// file in the repo, not just other changed files. Without this
/// mode the scan would miss copy-paste from an unchanged peer.
pub async fn run_duplication_scan_scoped(
    source: Arc<dyn TreeSource>,
    scope_filter: Arc<dyn FileFilter>,
    timeout: Option<std::time::Duration>,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let timeout = timeout.unwrap_or(DEFAULT_SCAN_TIMEOUT);
    let task = tokio::task::spawn_blocking(move || -> Result<_, CodeQualityError> {
        // The corpus deliberately uses AllFiles — the scope filter
        // determines which findings we keep, NOT which files we
        // walk. A copy-paste from an unchanged file only surfaces
        // when that unchanged file is in the corpus.
        let all = AllFiles;
        let corpus = collect_corpus(source.as_ref(), &all)?;
        let inputs: Vec<(String, String)> = corpus
            .into_iter()
            .filter(|(p, _)| oxplow_code_metrics::is_supported_path(Path::new(p)))
            .collect();
        let scope: BTreeSet<String> = inputs
            .iter()
            .map(|(p, _)| p.clone())
            .filter(|p| scope_filter.keep(p))
            .collect();
        let blocks = detect_duplicates_scoped(inputs, &scope, DupOptions::default());
        Ok(blocks_to_findings(blocks))
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(inner)) => inner,
        Ok(Err(join_err)) => Err(CodeQualityError::Task(format!("duplication task: {join_err}"))),
        Err(_) => Err(CodeQualityError::Timeout(timeout)),
    }
}

/// Backward-compat thin wrapper for callers that still pass a
/// project_dir: defaults to `DiskTreeSource` + `AllFiles`. New
/// callers should construct the source/filter explicitly via
/// [`run_duplication_scan_with`].
pub async fn run_duplication_scan(
    project_dir: &Path,
    opts: RunOptions,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let source: Arc<dyn TreeSource> = Arc::new(DiskTreeSource::new(project_dir.to_path_buf()));
    let filter: Arc<dyn FileFilter> = if opts.files.is_empty() {
        Arc::new(AllFiles)
    } else {
        Arc::new(oxplow_tree_source::ExplicitPaths::new(opts.files.iter().cloned()))
    };
    run_duplication_scan_with(source, filter, opts.timeout).await
}

fn blocks_to_findings(blocks: Vec<oxplow_code_dup::DuplicateBlock>) -> Vec<CodeQualityFinding> {
    let mut out = Vec::with_capacity(blocks.len() * 2);
    for b in blocks {
        let extra_a = format!(
            r#"{{"peerPath":{:?},"peerStartLine":{},"peerEndLine":{}}}"#,
            b.b_path, b.b_start_line, b.b_end_line
        );
        out.push(CodeQualityFinding {
            path: b.a_path.clone(),
            start_line: b.a_start_line,
            end_line: b.a_end_line,
            kind: "duplicate-block".into(),
            metric_value: b.line_count as f64,
            extra_json: Some(extra_a),
        });
        let extra_b = format!(
            r#"{{"peerPath":{:?},"peerStartLine":{},"peerEndLine":{}}}"#,
            b.a_path, b.a_start_line, b.a_end_line
        );
        out.push(CodeQualityFinding {
            path: b.b_path,
            start_line: b.b_start_line,
            end_line: b.b_end_line,
            kind: "duplicate-block".into(),
            metric_value: b.line_count as f64,
            extra_json: Some(extra_b),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metrics_scan_emits_findings_for_a_supported_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sample.rs");
        std::fs::write(
            &file,
            r#"
fn classify(x: i32) -> &'static str {
    if x > 0 { "pos" } else if x < 0 { "neg" } else { "zero" }
}
"#,
        )
        .unwrap();
        let findings = run_metrics_scan(dir.path(), RunOptions::default())
            .await
            .unwrap();
        assert!(findings.iter().any(|f| f.kind == "complexity"));
        assert!(findings.iter().any(|f| f.kind == "function-length"));
        assert!(findings.iter().any(|f| f.kind == "parameter-count"));
    }

    #[tokio::test]
    async fn metrics_scan_skips_unsupported_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# heading").unwrap();
        let findings = run_metrics_scan(dir.path(), RunOptions::default())
            .await
            .unwrap();
        assert!(findings.is_empty());
    }

    #[tokio::test]
    async fn duplication_scan_emits_paired_findings_for_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
        std::fs::write(dir.path().join("a.rs"), body).unwrap();
        std::fs::write(dir.path().join("b.rs"), body).unwrap();
        let findings = run_duplication_scan(dir.path(), RunOptions::default())
            .await
            .unwrap();
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(
            dups.len() >= 2,
            "expected at least one paired duplicate, got {:?}",
            findings
        );
        // Each finding's extra_json must carry the peer side as flat
        // keys (peerPath / peerStartLine / peerEndLine) — the panel
        // renderer reads them directly off `extra` without unwrapping
        // a nested object.
        for f in &dups {
            let raw = f.extra_json.as_deref().expect("extra_json present");
            let parsed: serde_json::Value = serde_json::from_str(raw)
                .expect("extra_json parses as JSON");
            assert!(
                parsed.get("peerPath").and_then(|v| v.as_str()).is_some(),
                "expected peerPath in extra_json, got {raw}"
            );
            assert!(
                parsed.get("peerStartLine").and_then(|v| v.as_i64()).is_some(),
                "expected peerStartLine in extra_json, got {raw}"
            );
            assert!(
                parsed.get("peerEndLine").and_then(|v| v.as_i64()).is_some(),
                "expected peerEndLine in extra_json, got {raw}"
            );
        }
    }

    #[tokio::test]
    async fn metrics_scan_returns_timeout_error_when_budget_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn x() {}").unwrap();
        // 1ns budget — the spawn_blocking task can't complete that fast,
        // even with one trivial file.
        let opts = RunOptions {
            files: Vec::new(),
            timeout: Some(std::time::Duration::from_nanos(1)),
        };
        let err = run_metrics_scan(dir.path(), opts).await.unwrap_err();
        assert!(
            matches!(err, CodeQualityError::Timeout(_)),
            "expected Timeout, got {err:?}"
        );
    }

    /// Integration: a small multi-file fixture exercises both scanners
    /// end-to-end (file walker + relative-path stripping + finding fan-out
    /// + cross-doc dup matching all in one pass).
    #[tokio::test]
    async fn end_to_end_fixture_scan_metrics_and_duplication() {
        let dir = tempfile::tempdir().unwrap();
        // File A: a function with branching.
        std::fs::write(
            dir.path().join("a.rs"),
            r#"
fn process(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#,
        )
        .unwrap();
        // File B: same function body with renamed identifiers (clone).
        std::fs::write(
            dir.path().join("b.rs"),
            r#"
fn handle(values: Vec<i32>) -> Vec<i32> {
    let mut output = Vec::new();
    for v in values {
        if v > 0 {
            output.push(v * 2);
        } else if v < 0 {
            output.push(v * -1);
        } else {
            output.push(0);
        }
    }
    output
}
"#,
        )
        .unwrap();
        // File C: an unsupported language — must not appear anywhere.
        std::fs::write(dir.path().join("README.md"), "# heading\nsome text\n").unwrap();
        // Nested skipped dir — must not be scanned.
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/should_skip.rs"), "fn x() {}").unwrap();

        let metrics = run_metrics_scan(dir.path(), RunOptions::default())
            .await
            .unwrap();
        // Two functions × three metric kinds = 6 findings.
        let function_kinds: Vec<_> = metrics.iter().map(|f| f.kind.as_str()).collect();
        assert!(function_kinds.iter().filter(|k| **k == "complexity").count() == 2);
        assert!(function_kinds.iter().filter(|k| **k == "function-length").count() == 2);
        // Both functions take one argument.
        assert!(function_kinds.iter().filter(|k| **k == "parameter-count").count() == 2);
        // Paths are repo-relative (not absolute, not under target/).
        for f in &metrics {
            assert!(!f.path.starts_with('/'), "leaked absolute path: {}", f.path);
            assert!(!f.path.contains("target/"), "scanned skipped dir: {}", f.path);
        }
        assert!(metrics.iter().all(|f| f.path == "a.rs" || f.path == "b.rs"));

        let duplication = run_duplication_scan(dir.path(), RunOptions::default())
            .await
            .unwrap();
        let dups: Vec<_> = duplication
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(dups.len() >= 2, "expected paired duplicate, got {duplication:?}");
        for f in &duplication {
            assert!(!f.path.starts_with('/'));
            assert!(!f.path.contains("target/"));
        }
    }

    #[tokio::test]
    async fn duplication_scan_emits_nothing_for_unique_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.rs"),
            "fn unrelated() { println!(\"hi\"); }",
        )
        .unwrap();
        let findings = run_duplication_scan(dir.path(), RunOptions::default())
            .await
            .unwrap();
        assert!(findings.is_empty());
    }

    /// The dup scan must read content from the supplied tree source,
    /// not from disk. Set up a git repo whose committed `a.rs` and
    /// `b.rs` are intentional clones of each other, then mutate the
    /// disk versions to be unique. A scan against `HEAD` via
    /// `GitTreeSource` should still report the duplicates; a scan
    /// against `Disk` would not.
    /// The scoped runner walks the whole tree but only surfaces
    /// findings whose A side is in scope. Verifies the
    /// change-analysis "compare changed files against everything"
    /// semantic.
    #[tokio::test]
    async fn duplication_scan_scoped_finds_clones_in_unchanged_peers() {
        use oxplow_tree_source::{DiskTreeSource, ExplicitPaths};
        let dir = tempfile::tempdir().unwrap();
        let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
        std::fs::write(dir.path().join("changed.rs"), body).unwrap();
        std::fs::write(dir.path().join("untouched.rs"), body).unwrap();
        let source: Arc<dyn TreeSource> =
            Arc::new(DiskTreeSource::new(dir.path().to_path_buf()));
        // Scope = only the changed file. The peer (untouched.rs) is
        // NOT in scope but must still participate as a match
        // target.
        let scope: Arc<dyn FileFilter> =
            Arc::new(ExplicitPaths::new(vec!["changed.rs".to_string()]));
        let findings = run_duplication_scan_scoped(source, scope, None)
            .await
            .unwrap();
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(
            !dups.is_empty(),
            "expected dup findings between changed and untouched, got {findings:?}",
        );
        // Every finding's anchor (path) is the scope file; the peer
        // (extra.peerPath) is the unchanged file. The flat
        // findings list emits one record per side, so the scope
        // file shows up at least once.
        assert!(
            findings.iter().any(|f| f.path == "changed.rs"),
            "expected changed.rs to anchor at least one finding"
        );
    }

    /// Same-file pairs (file matched against itself, two regions
    /// in one file) must be dropped by the scoped runner.
    #[tokio::test]
    async fn duplication_scan_scoped_drops_same_file_self_match() {
        use oxplow_tree_source::{DiskTreeSource, ExplicitPaths};
        let dir = tempfile::tempdir().unwrap();
        let body_with_repeat = r#"
fn case_a(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 { out.push(item * 2); }
        else if item < 0 { out.push(item * -1); }
        else { out.push(0); }
    }
    out
}

fn case_b(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 { out.push(item * 2); }
        else if item < 0 { out.push(item * -1); }
        else { out.push(0); }
    }
    out
}
"#;
        std::fs::write(dir.path().join("only.rs"), body_with_repeat).unwrap();
        let source: Arc<dyn TreeSource> =
            Arc::new(DiskTreeSource::new(dir.path().to_path_buf()));
        let scope: Arc<dyn FileFilter> =
            Arc::new(ExplicitPaths::new(vec!["only.rs".to_string()]));
        let findings = run_duplication_scan_scoped(source, scope, None)
            .await
            .unwrap();
        // Even if the engine surfaces in-file matches, the scoped
        // runner's same-path filter must drop them.
        for f in &findings {
            let raw = f.extra_json.as_deref().unwrap_or("{}");
            let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
            let peer = parsed.get("peerPath").and_then(|v| v.as_str()).unwrap_or("");
            assert_ne!(peer, f.path, "same-file pair leaked: {f:?}");
        }
    }

    #[tokio::test]
    async fn duplication_scan_reads_from_tree_source_not_disk() {
        use oxplow_tree_source::{AllFiles, GitTreeSource};
        use std::process::Command;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(path)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed: {:?}", out);
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
        std::fs::write(path.join("a.rs"), body).unwrap();
        std::fs::write(path.join("b.rs"), body).unwrap();
        run(&["add", "a.rs", "b.rs"]);
        run(&["commit", "-q", "-m", "first"]);
        // After commit: stomp the disk versions so they're no longer
        // duplicates. Any scan that secretly reads disk would now
        // emit zero findings.
        std::fs::write(path.join("a.rs"), "fn unique_a() {}").unwrap();
        std::fs::write(path.join("b.rs"), "fn unique_b() {}").unwrap();

        let source: Arc<dyn TreeSource> = Arc::new(GitTreeSource::new(path, "HEAD"));
        let filter: Arc<dyn FileFilter> = Arc::new(AllFiles);
        let findings = run_duplication_scan_with(source, filter, None).await.unwrap();
        let dups: Vec<_> = findings
            .iter()
            .filter(|f| f.kind == "duplicate-block")
            .collect();
        assert!(
            dups.len() >= 2,
            "expected dup findings from HEAD content, got {findings:?}"
        );
    }
}

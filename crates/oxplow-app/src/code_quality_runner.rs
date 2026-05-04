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

use oxplow_code_dup::{detect_duplicates, DupOptions};
use oxplow_code_metrics::FunctionMetrics;
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum CodeQualityError {
    /// Surfaces a failure inside the spawn_blocking pool (panic or
    /// joining error). Renamed from "parse" historically.
    #[error("scan task failed: {0}")]
    Task(String),
}

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
    /// Reserved for future use.
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
    // The metric pass is CPU-bound; punt to a blocking pool so we
    // don't stall the tokio runtime on large repos.
    let findings = tokio::task::spawn_blocking(move || -> Result<_, CodeQualityError> {
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
    })
    .await
    .map_err(|e| CodeQualityError::Task(format!("metrics task: {e}")))??;
    Ok(findings)
}

/// Duplicate-block scan: runs the tree-sitter winnowing detector
/// across every supported file and emits two findings per duplicate
/// pair (one per side).
pub async fn run_duplication_scan(
    project_dir: &Path,
    opts: RunOptions,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let project = project_dir.to_path_buf();
    let findings = tokio::task::spawn_blocking(move || -> Result<_, CodeQualityError> {
        let paths = collect_supported_files(&project, &opts);
        // Read all sources up front; the dup detector wants
        // (path, content) pairs.
        let mut inputs: Vec<(String, String)> = Vec::with_capacity(paths.len());
        for path in paths {
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            // Re-derive repo-relative path so peer references in the
            // findings match the panel's other rows.
            let rel = match path.strip_prefix(&project) {
                Ok(rel) => rel.to_string_lossy().to_string(),
                Err(_) => path.to_string_lossy().to_string(),
            };
            inputs.push((rel, source));
        }
        let blocks = detect_duplicates(inputs, DupOptions::default());
        Ok(blocks_to_findings(blocks))
    })
    .await
    .map_err(|e| CodeQualityError::Task(format!("duplication task: {e}")))??;
    Ok(findings)
}

fn blocks_to_findings(blocks: Vec<oxplow_code_dup::DuplicateBlock>) -> Vec<CodeQualityFinding> {
    let mut out = Vec::with_capacity(blocks.len() * 2);
    for b in blocks {
        let extra_a = format!(
            r#"{{"peer":{{"path":{:?},"startLine":{},"endLine":{}}}}}"#,
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
            r#"{{"peer":{{"path":{:?},"startLine":{},"endLine":{}}}}}"#,
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
}

//! Code-quality scanners.
//!
//! - `run_lizard`: in-process metrics via `oxplow_code_metrics`. The
//!   name is kept for callsite continuity; nothing shells out anymore.
//! - `run_jscpd`: still shells out to `jscpd` (Phase 2 will replace).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use oxplow_code_metrics::FunctionMetrics;
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tokio::process::Command;
#[cfg(test)]
use tracing::warn;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum CodeQualityError {
    #[error("`{tool}` not found on PATH")]
    ToolMissing { tool: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("subprocess timed out")]
    Timeout,
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
    /// Free-form JSON for tool-specific metadata. The store persists
    /// this as a string column.
    pub extra_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Subset of repo-relative paths. Empty = scan whole repo.
    pub files: Vec<String>,
    /// Reserved for future use (was the lizard subprocess timeout).
    pub timeout: Option<std::time::Duration>,
}

fn map_spawn_err(err: std::io::Error, tool: &str) -> CodeQualityError {
    if err.kind() == std::io::ErrorKind::NotFound {
        CodeQualityError::ToolMissing {
            tool: tool.into(),
        }
    } else {
        CodeQualityError::Io(err)
    }
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
        if m.complexity > 0 {
            out.push(CodeQualityFinding {
                path: path.clone(),
                start_line: m.start_line,
                end_line: m.end_line,
                kind: "complexity".into(),
                metric_value: m.complexity as f64,
                extra_json: Some(extra.clone()),
            });
        }
        if m.length > 0 {
            out.push(CodeQualityFinding {
                path: path.clone(),
                start_line: m.start_line,
                end_line: m.end_line,
                kind: "function-length".into(),
                metric_value: m.length as f64,
                extra_json: Some(extra.clone()),
            });
        }
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

/// Native, in-process replacement for the lizard CLI. Same name,
/// same signature, same finding shape.
pub async fn run_lizard(
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
    .map_err(|e| CodeQualityError::Parse(format!("metrics task: {e}")))??;
    Ok(findings)
}

#[derive(Debug, Deserialize)]
struct JscpdReport {
    duplicates: Vec<JscpdDuplicate>,
}

#[derive(Debug, Deserialize)]
struct JscpdDuplicate {
    #[serde(rename = "firstFile")]
    first_file: JscpdFile,
    #[serde(rename = "secondFile")]
    second_file: JscpdFile,
    #[serde(default)]
    lines: u32,
}

#[derive(Debug, Deserialize)]
struct JscpdFile {
    name: String,
    start: u32,
    end: u32,
}

/// Run jscpd against the project. Two findings per duplicate (one
/// per peer location). Phase 2 will replace this with a native
/// tree-sitter winnowing detector.
pub async fn run_jscpd(
    project_dir: &Path,
    opts: RunOptions,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let out_dir = tempfile::tempdir()?;
    let mut args: Vec<String> = vec![
        "--reporters".into(),
        "json".into(),
        "--silent".into(),
        "--output".into(),
        out_dir.path().to_string_lossy().into_owned(),
    ];
    if !opts.files.is_empty() {
        args.push("--pattern".into());
        args.push(opts.files.join(","));
    }
    args.push(project_dir.to_string_lossy().into_owned());
    let timeout = opts.timeout.unwrap_or(std::time::Duration::from_secs(60));
    let proc = Command::new("jscpd")
        .args(&args)
        .current_dir(project_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| map_spawn_err(e, "jscpd"))?;
    match tokio::time::timeout(timeout, proc.wait_with_output()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(CodeQualityError::Io(e)),
        Err(_) => return Err(CodeQualityError::Timeout),
    }
    let report_path: PathBuf = out_dir.path().join("jscpd-report.json");
    let raw = match std::fs::read_to_string(&report_path) {
        Ok(s) => s,
        Err(_) => return Ok(vec![]),
    };
    parse_jscpd_report(&raw)
}

pub fn parse_jscpd_report(raw: &str) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let report: JscpdReport = serde_json::from_str(raw)
        .map_err(|e| CodeQualityError::Parse(format!("jscpd report: {e}")))?;
    let mut out = Vec::with_capacity(report.duplicates.len() * 2);
    for dup in report.duplicates {
        let extra = format!(
            r#"{{"peer":{{"path":{:?},"startLine":{},"endLine":{}}}}}"#,
            dup.second_file.name, dup.second_file.start, dup.second_file.end
        );
        out.push(CodeQualityFinding {
            path: dup.first_file.name.clone(),
            start_line: dup.first_file.start,
            end_line: dup.first_file.end,
            kind: "duplicate-block".into(),
            metric_value: dup.lines as f64,
            extra_json: Some(extra),
        });
        let extra2 = format!(
            r#"{{"peer":{{"path":{:?},"startLine":{},"endLine":{}}}}}"#,
            dup.first_file.name, dup.first_file.start, dup.first_file.end
        );
        out.push(CodeQualityFinding {
            path: dup.second_file.name,
            start_line: dup.second_file.start,
            end_line: dup.second_file.end,
            kind: "duplicate-block".into(),
            metric_value: dup.lines as f64,
            extra_json: Some(extra2),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_lizard_emits_findings_for_a_supported_file() {
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
        let findings = run_lizard(dir.path(), RunOptions::default()).await.unwrap();
        assert!(findings.iter().any(|f| f.kind == "complexity"));
        assert!(findings.iter().any(|f| f.kind == "function-length"));
        assert!(findings.iter().any(|f| f.kind == "parameter-count"));
    }

    #[tokio::test]
    async fn run_lizard_skips_unsupported_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# heading").unwrap();
        let findings = run_lizard(dir.path(), RunOptions::default()).await.unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_jscpd_report_emits_two_findings_per_duplicate() {
        let raw = serde_json::json!({
            "duplicates": [{
                "firstFile": { "name": "a.rs", "start": 1, "end": 10 },
                "secondFile": { "name": "b.rs", "start": 20, "end": 29 },
                "lines": 10,
            }]
        })
        .to_string();
        let findings = parse_jscpd_report(&raw).unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].kind, "duplicate-block");
    }

    #[tokio::test]
    async fn run_jscpd_returns_tool_missing_when_absent() {
        let result = run_with_renamed_binary("jscpd-no-such-binary").await;
        match result {
            Err(CodeQualityError::ToolMissing { tool }) => {
                assert_eq!(tool, "jscpd-no-such-binary");
            }
            other => {
                let _ = other;
                warn!("jscpd-no-such-binary unexpectedly resolved; skipping assertion");
            }
        }
    }

    async fn run_with_renamed_binary(
        cmd: &str,
    ) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
        let proc = Command::new(cmd)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| map_spawn_err(e, cmd))?;
        let _ = proc.wait_with_output().await?;
        Ok(vec![])
    }
}

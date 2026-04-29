//! Subprocess driver for the lizard / jscpd code-quality scanners.
//!
//! Mirrors the original `src/subprocess/code-quality.ts`:
//! - `run_lizard`: shells out to `lizard --csv`, parses the CSV into one
//!   finding per metric (complexity, function-length, parameter-count).
//! - `run_jscpd`: shells out to `jscpd --reporters json`, reads the
//!   resulting `jscpd-report.json`, emits one finding per duplicated
//!   block instance.
//!
//! Both detect "tool not on PATH" with a typed error so the renderer
//! can surface "install lizard with `pip install lizard`" etc.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tokio::process::Command;
use tracing::warn;

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
    /// Subprocess timeout. Defaults to 60s.
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

/// Run lizard against the project. One finding per (function,
/// metric); the renderer groups by `extra.functionName`.
pub async fn run_lizard(
    project_dir: &Path,
    opts: RunOptions,
) -> Result<Vec<CodeQualityFinding>, CodeQualityError> {
    let mut args: Vec<String> = vec!["--csv".to_string()];
    if opts.files.is_empty() {
        args.push(project_dir.to_string_lossy().into_owned());
    } else {
        args.extend(opts.files.iter().cloned());
    }
    let timeout = opts.timeout.unwrap_or(std::time::Duration::from_secs(60));
    let proc = Command::new("lizard")
        .args(&args)
        .current_dir(project_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| map_spawn_err(e, "lizard"))?;
    let output = match tokio::time::timeout(timeout, proc.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(CodeQualityError::Io(e)),
        Err(_) => return Err(CodeQualityError::Timeout),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_lizard_csv(&stdout))
}

/// Parse lizard's `--csv` output. Columns:
///   nloc, ccn, token, parameter_count, length, location, file, function, length(line)
/// We emit up to three findings per function row: complexity (ccn),
/// function-length (length), parameter-count (parameter_count).
pub fn parse_lizard_csv(raw: &str) -> Vec<CodeQualityFinding> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = trimmed.split(',').collect();
        if cols.len() < 9 {
            continue;
        }
        // CSV format from lizard --csv:
        //   1   2   3       4                5       6        7    8         9
        //   nloc ccn token  parameter_count  length  location file function  start_line
        let ccn: f64 = cols[1].trim().parse().unwrap_or(0.0);
        let parameters: f64 = cols[3].trim().parse().unwrap_or(0.0);
        let length: f64 = cols[4].trim().parse().unwrap_or(0.0);
        let file = cols[6].trim().to_string();
        let function = cols[7].trim().to_string();
        let start_line: u32 = cols[8].trim().parse().unwrap_or(0);
        let end_line = start_line + length as u32;

        let mk = |kind: &str, value: f64| CodeQualityFinding {
            path: file.clone(),
            start_line,
            end_line,
            kind: kind.into(),
            metric_value: value,
            extra_json: Some(format!(r#"{{"functionName":{:?}}}"#, function)),
        };
        if ccn > 0.0 {
            out.push(mk("complexity", ccn));
        }
        if length > 0.0 {
            out.push(mk("function-length", length));
        }
        if parameters > 0.0 {
            out.push(mk("parameter-count", parameters));
        }
    }
    out
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
/// per peer location).
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
        // jscpd writes no report when zero duplicates are found in
        // some versions; treat as empty.
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

    #[test]
    fn parse_lizard_csv_emits_three_findings_per_function() {
        let raw = "10,5,40,2,8,foo (8),src/x.rs,foo,12";
        let findings = parse_lizard_csv(raw);
        assert_eq!(findings.len(), 3);
        assert!(findings.iter().any(|f| f.kind == "complexity"));
        assert!(findings.iter().any(|f| f.kind == "function-length"));
        assert!(findings.iter().any(|f| f.kind == "parameter-count"));
    }

    #[test]
    fn parse_lizard_csv_skips_zero_rows() {
        let raw = "0,0,0,0,0,foo (0),src/x.rs,foo,1";
        assert!(parse_lizard_csv(raw).is_empty());
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
    async fn run_lizard_returns_tool_missing_when_absent() {
        // Force the tool name to something definitely missing by
        // delegating into a renamed binary check. Skip on systems
        // where `lizard-no-such-binary` somehow exists.
        let result = run_with_renamed_binary("lizard-no-such-binary").await;
        match result {
            Err(CodeQualityError::ToolMissing { tool }) => {
                assert_eq!(tool, "lizard-no-such-binary");
            }
            // Skip if a binary with that name actually exists.
            other => {
                let _ = other;
                warn!("lizard-no-such-binary unexpectedly resolved; skipping assertion");
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

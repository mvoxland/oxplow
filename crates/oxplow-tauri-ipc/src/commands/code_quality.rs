use oxplow_app::code_quality_runner::{
    parse_lizard_csv, run_jscpd, run_lizard, CodeQualityFinding as RunnerFinding, RunOptions,
};
use oxplow_app::{CodeQualityScanPhase, OxplowEvent};
use oxplow_db::{CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_code_quality_scans(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<CodeQualityScan>, IpcError> {
    Ok(state.code_quality_store.list_scans(limit as usize).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_code_quality_findings(
    state: tauri::State<'_, AppState>,
    scan_id: i64,
) -> Result<Vec<CodeQualityFinding>, IpcError> {
    Ok(state.code_quality_store.list_findings(scan_id).await?)
}

/// Run a fresh lizard or jscpd scan, persist its findings, and
/// return the scan id. Tool name is one of `"lizard"` / `"jscpd"`.
/// `scope` is a free-form label (typically `"workspace"`).
#[tauri::command]
#[specta::specta]
pub async fn run_code_quality_scan(
    state: tauri::State<'_, AppState>,
    tool: String,
    scope: String,
    files: Option<Vec<String>>,
) -> Result<i64, IpcError> {
    let project = state.layout.project_dir.clone();
    let opts = RunOptions {
        files: files.unwrap_or_default(),
        timeout: None,
    };
    let scan_id = state
        .code_quality_store
        .create_scan(&tool, &scope)
        .await?;
    state.events.emit(OxplowEvent::CodeQualityScanned {
        stream_id: None,
        scan_id,
        tool: tool.clone(),
        scope: scope.clone(),
        phase: CodeQualityScanPhase::Started,
    });
    let findings_result = match tool.as_str() {
        "lizard" => run_lizard(&project, opts).await,
        "jscpd" => run_jscpd(&project, opts).await,
        other => {
            state
                .code_quality_store
                .finish_scan(
                    scan_id,
                    CodeQualityScanStatus::Failed,
                    Some(format!("unknown tool: {other}")),
                )
                .await?;
            state.events.emit(OxplowEvent::CodeQualityScanned {
                stream_id: None,
                scan_id,
                tool: tool.clone(),
                scope: scope.clone(),
                phase: CodeQualityScanPhase::Failed,
            });
            return Err(IpcError::invalid(format!("unknown code quality tool: {other}")));
        }
    };
    match findings_result {
        Ok(findings) => {
            for f in findings {
                state
                    .code_quality_store
                    .append_finding(
                        scan_id,
                        oxplow_db::CodeQualityFinding {
                            id: 0,
                            scan_id,
                            path: f.path,
                            start_line: f.start_line as i32,
                            end_line: f.end_line as i32,
                            kind: f.kind,
                            metric_value: f.metric_value,
                            extra_json: f.extra_json,
                        },
                    )
                    .await?;
            }
            state
                .code_quality_store
                .finish_scan(scan_id, CodeQualityScanStatus::Done, None)
                .await?;
            state.events.emit(OxplowEvent::CodeQualityScanned {
                stream_id: None,
                scan_id,
                tool: tool.clone(),
                scope: scope.clone(),
                phase: CodeQualityScanPhase::Completed,
            });
        }
        Err(e) => {
            state
                .code_quality_store
                .finish_scan(
                    scan_id,
                    CodeQualityScanStatus::Failed,
                    Some(e.to_string()),
                )
                .await?;
            state.events.emit(OxplowEvent::CodeQualityScanned {
                stream_id: None,
                scan_id,
                tool: tool.clone(),
                scope: scope.clone(),
                phase: CodeQualityScanPhase::Failed,
            });
            return Err(IpcError::internal(e.to_string()));
        }
    }
    Ok(scan_id)
}

/// One file's content at one side of the diff. `content == None` means
/// the file did not exist on that side (e.g. add/delete).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AnalyzeFileSpec {
    pub path: String,
    pub base_content: Option<String>,
    pub head_content: Option<String>,
}

/// Function metadata produced by lizard for one (path, side) pair.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFunction {
    pub name: String,
    pub start_line: u32,
    pub length: u32,
    pub complexity: f64,
    pub parameter_count: u32,
    pub nloc: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFileSide {
    pub path: String,
    /// `"base"` or `"head"`.
    pub side: String,
    pub functions: Vec<AnalyzedFunction>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzeFunctionsResult {
    pub sides: Vec<AnalyzedFileSide>,
    /// `Some(msg)` when lizard isn't on PATH; the renderer surfaces an
    /// inline install hint and treats `sides` as empty.
    pub tool_missing: Option<String>,
}

/// Run lizard against base- and head-side contents of a set of files
/// to compute per-function metadata for the Change Analysis dashboard.
///
/// Strategy: write each `(path, side)` pair into a temp directory at
/// `<tmp>/<side>/<path>` so the file extension is preserved (lizard's
/// language detection is extension-driven), invoke `lizard --csv` once
/// over the temp root, then route findings back by parsing the
/// `side` segment of the temp path.
#[tauri::command]
#[specta::specta]
pub async fn analyze_functions_at_refs(
    files: Vec<AnalyzeFileSpec>,
) -> Result<AnalyzeFunctionsResult, IpcError> {
    if files.is_empty() {
        return Ok(AnalyzeFunctionsResult {
            sides: Vec::new(),
            tool_missing: None,
        });
    }
    let tmp = tempfile::tempdir().map_err(|e| IpcError::internal(e.to_string()))?;
    let root = tmp.path().to_path_buf();

    // Write all (path, side) pairs into the tempdir, mirroring the
    // repo-relative path so lizard sees the original extension.
    for spec in &files {
        for (side, content) in [
            ("base", spec.base_content.as_deref()),
            ("head", spec.head_content.as_deref()),
        ] {
            let Some(text) = content else { continue };
            let dest: PathBuf = root.join(side).join(&spec.path);
            if let Some(parent) = dest.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Err(IpcError::internal(e.to_string()));
                }
            }
            if let Err(e) = tokio::fs::write(&dest, text).await {
                return Err(IpcError::internal(e.to_string()));
            }
        }
    }

    // Invoke lizard once over the temp root.
    let proc = Command::new("lizard")
        .args(["--csv", root.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let proc = match proc {
        Ok(p) => p,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AnalyzeFunctionsResult {
                sides: Vec::new(),
                tool_missing: Some("lizard".to_string()),
            });
        }
        Err(err) => return Err(IpcError::internal(err.to_string())),
    };
    let output = proc
        .wait_with_output()
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let csv = String::from_utf8_lossy(&output.stdout).to_string();
    let raw: Vec<RunnerFinding> = parse_lizard_csv(&csv);

    // Group findings by (side, path) and collapse the three
    // per-function rows (complexity / function-length / parameter-count)
    // into a single AnalyzedFunction record.
    use std::collections::BTreeMap;
    let root_str = root.to_string_lossy().to_string();
    type Bucket = BTreeMap<(String, String, String, u32), AnalyzedFunction>;
    let mut bucket: Bucket = BTreeMap::new();
    for f in raw {
        // f.path is the absolute path lizard saw; strip the temp root
        // prefix and split off the leading "base/" or "head/" segment.
        let stripped = match f.path.strip_prefix(&root_str) {
            Some(s) => s.trim_start_matches('/').trim_start_matches('\\'),
            None => continue,
        };
        let mut parts = stripped.splitn(2, |c: char| c == '/' || c == '\\');
        let side = parts.next().unwrap_or("").to_string();
        let rel_path = parts.next().unwrap_or("").to_string();
        if rel_path.is_empty() || (side != "base" && side != "head") {
            continue;
        }
        let function_name = function_name_from_extra(f.extra_json.as_deref()).unwrap_or_default();
        let key = (side.clone(), rel_path.clone(), function_name.clone(), f.start_line);
        let entry = bucket.entry(key).or_insert(AnalyzedFunction {
            name: function_name,
            start_line: f.start_line,
            length: 0,
            complexity: 0.0,
            parameter_count: 0,
            nloc: 0,
        });
        match f.kind.as_str() {
            "complexity" => entry.complexity = f.metric_value,
            "function-length" => entry.length = f.metric_value as u32,
            "parameter-count" => entry.parameter_count = f.metric_value as u32,
            _ => {}
        }
    }

    // Re-bucket into per-(side, path) AnalyzedFileSide rows.
    let mut sides_map: BTreeMap<(String, String), Vec<AnalyzedFunction>> = BTreeMap::new();
    for ((side, path, _name, _start), func) in bucket {
        sides_map.entry((side, path)).or_default().push(func);
    }

    // Suppress unused-import warning when no scan-store path runs.
    let _ = std::any::type_name::<RunOptions>();

    let mut sides: Vec<AnalyzedFileSide> = sides_map
        .into_iter()
        .map(|((side, path), functions)| AnalyzedFileSide {
            path,
            side,
            functions,
        })
        .collect();
    // Stable order: by path, then side.
    sides.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.side.cmp(&b.side)));

    Ok(AnalyzeFunctionsResult {
        sides,
        tool_missing: None,
    })
}

fn function_name_from_extra(extra: Option<&str>) -> Option<String> {
    let raw = extra?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value
        .get("functionName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

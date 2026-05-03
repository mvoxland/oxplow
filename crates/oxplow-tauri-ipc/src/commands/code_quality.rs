use oxplow_app::code_quality_runner::{
    run_jscpd, run_lizard, CodeQualityFinding as RunnerFinding, RunOptions,
};
use oxplow_app::{CodeQualityScanPhase, OxplowEvent};
use oxplow_code_metrics::{analyze_file, FunctionMetrics};
use oxplow_db::{CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus};
use serde::{Deserialize, Serialize};
use specta::Type;

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

/// Run a fresh metrics or duplication scan, persist findings, and
/// return the scan id. Tool name is one of `"lizard"` (in-process
/// metrics) / `"jscpd"` (subprocess; replaced in Phase 2).
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

/// Function metadata for one (path, side) pair.
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
    /// Always `None` now that the implementation is in-process. Kept
    /// for renderer back-compat — will be removed once the UI stops
    /// branching on it.
    pub tool_missing: Option<String>,
}

/// Compute per-function metadata for the Change Analysis dashboard,
/// for both sides of the diff. Pure in-process call: walks each
/// (path, content) pair through tree-sitter, no subprocess, no
/// tempdir, no install dependency.
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
    let sides = tokio::task::spawn_blocking(move || analyze_sides(files))
        .await
        .map_err(|e| IpcError::internal(format!("analyze task: {e}")))?;
    Ok(AnalyzeFunctionsResult {
        sides,
        tool_missing: None,
    })
}

fn analyze_sides(files: Vec<AnalyzeFileSpec>) -> Vec<AnalyzedFileSide> {
    let mut out = Vec::new();
    for spec in files {
        if let Some(content) = spec.base_content.as_deref() {
            out.push(AnalyzedFileSide {
                path: spec.path.clone(),
                side: "base".into(),
                functions: to_analyzed(analyze_file(&spec.path, content)),
            });
        }
        if let Some(content) = spec.head_content.as_deref() {
            out.push(AnalyzedFileSide {
                path: spec.path.clone(),
                side: "head".into(),
                functions: to_analyzed(analyze_file(&spec.path, content)),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.side.cmp(&b.side)));
    out
}

fn to_analyzed(metrics: Vec<FunctionMetrics>) -> Vec<AnalyzedFunction> {
    metrics
        .into_iter()
        .map(|m| AnalyzedFunction {
            name: m.name,
            start_line: m.start_line,
            length: m.length,
            complexity: m.complexity as f64,
            parameter_count: m.parameter_count,
            // We don't compute non-comment line count separately;
            // approximate as length. Renderer treats it as informational.
            nloc: m.length,
        })
        .collect()
}

#[allow(dead_code)]
fn _runner_finding_kept_in_scope(_: RunnerFinding) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn analyze_functions_returns_function_for_each_side() {
        let files = vec![AnalyzeFileSpec {
            path: "src/foo.rs".into(),
            base_content: Some("fn a() {}".into()),
            head_content: Some(
                "fn a() { if true { 1; } }".into(),
            ),
        }];
        let result = analyze_functions_at_refs(files).await.unwrap();
        assert_eq!(result.tool_missing, None);
        assert_eq!(result.sides.len(), 2);
        let head = result
            .sides
            .iter()
            .find(|s| s.side == "head")
            .unwrap();
        assert_eq!(head.functions.len(), 1);
        assert!(head.functions[0].complexity >= 2.0);
    }

    #[tokio::test]
    async fn analyze_functions_handles_added_file() {
        let files = vec![AnalyzeFileSpec {
            path: "src/new.py".into(),
            base_content: None,
            head_content: Some("def f(x):\n    return x\n".into()),
        }];
        let result = analyze_functions_at_refs(files).await.unwrap();
        assert_eq!(result.sides.len(), 1);
        assert_eq!(result.sides[0].side, "head");
    }

    #[tokio::test]
    async fn analyze_functions_skips_unsupported_languages() {
        let files = vec![AnalyzeFileSpec {
            path: "README.md".into(),
            base_content: Some("# old".into()),
            head_content: Some("# new".into()),
        }];
        let result = analyze_functions_at_refs(files).await.unwrap();
        // We still emit empty sides so the caller can see "we looked".
        assert_eq!(result.sides.len(), 2);
        assert!(result.sides[0].functions.is_empty());
    }
}

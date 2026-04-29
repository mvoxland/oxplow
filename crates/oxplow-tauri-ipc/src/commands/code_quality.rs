use oxplow_app::code_quality_runner::{run_jscpd, run_lizard, RunOptions};
use oxplow_db::{CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus};

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
            return Err(IpcError::internal(e.to_string()));
        }
    }
    Ok(scan_id)
}

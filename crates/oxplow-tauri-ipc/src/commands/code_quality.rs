use std::sync::Arc;

use oxplow_app::code_quality_runner::{
    run_duplication_scan, run_duplication_scan_scoped, run_metrics_scan, RunOptions,
};
use oxplow_app::{BackgroundTaskKind, CodeQualityScanPhase, OxplowEvent, StartInput};
use oxplow_code_metrics::{analyze_file, FunctionMetrics, Visibility};
use oxplow_db::{CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus};
use oxplow_tree_source::{
    AllFiles, DiskTreeSource, ExplicitPaths, FileFilter, GitTreeSource, TreeSource, TreeVersion,
};
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

/// Run a fresh code-quality scan, persist findings, and return the
/// scan id. `tool` selects the analysis kind: `"metrics"` for
/// per-function complexity/length/parameters, `"duplication"` for
/// duplicate-block detection. `scope` is a free-form label
/// (typically `"workspace"` or `"diff"`).
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
        dup_options: None,
    };
    let scan_id = state.code_quality_store.create_scan(&tool, &scope).await?;
    state.events.emit(OxplowEvent::CodeQualityScanned {
        stream_id: None,
        scan_id,
        tool: tool.clone(),
        scope: scope.clone(),
        phase: CodeQualityScanPhase::Started,
    });
    let findings_result = match tool.as_str() {
        "metrics" => run_metrics_scan(&project, opts).await,
        "duplication" => run_duplication_scan(&project, opts).await,
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
            return Err(IpcError::invalid(format!(
                "unknown code quality tool: {other}"
            )));
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
                .finish_scan(scan_id, CodeQualityScanStatus::Failed, Some(e.to_string()))
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

/// File filter the renderer can request: `all` (whole corpus) or an
/// explicit set of repo-relative paths. The serialized shape mirrors
/// the persisted `file_filter` column — callers pass `kind: "all"` or
/// `{ kind: "explicit", paths: [...] }`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FileFilterSpec {
    All,
    Explicit { paths: Vec<String> },
}

impl FileFilterSpec {
    fn fingerprint(&self) -> String {
        match self {
            FileFilterSpec::All => "all".into(),
            FileFilterSpec::Explicit { paths } => {
                use std::hash::{Hash, Hasher};
                let mut sorted: Vec<&String> = paths.iter().collect();
                sorted.sort();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for p in &sorted {
                    p.hash(&mut hasher);
                }
                format!("explicit:{:016x}", hasher.finish())
            }
        }
    }

    fn into_filter(self) -> Arc<dyn FileFilter> {
        match self {
            FileFilterSpec::All => Arc::new(AllFiles),
            FileFilterSpec::Explicit { paths } => Arc::new(ExplicitPaths::new(paths)),
        }
    }
}

/// Run a duplicate-block scan against `tree_version`, scoped by
/// `file_filter`. The corpus is the WHOLE tree at the requested
/// version — `file_filter` defines which files findings are
/// anchored to (the renderer's "side A"). A copy-paste from an
/// unchanged peer file surfaces because that peer is in the corpus
/// even though it's outside scope. Same-path matches (a file vs
/// itself) are dropped. Persists the scan row with the version +
/// filter columns so [`find_latest_done_scan`] can pick it up on
/// the next page load. Returns the scan id.
///
/// The renderer wires this to the "Scan now" button on the
/// duplication card. There is intentionally no auto-trigger:
/// scanning a commit's tree with libgit2 + tree-sitter is slow on a
/// large repo, so we keep it user-initiated until that becomes
/// interactive enough to make implicit.
#[tauri::command]
#[specta::specta]
pub async fn run_duplication_scan_at(
    state: tauri::State<'_, AppState>,
    tree_version: TreeVersion,
    file_filter: FileFilterSpec,
    scope: String,
) -> Result<i64, IpcError> {
    let project = state.layout.project_dir.clone();
    let kind_tag = tree_version.kind_tag().to_string();
    let value_str = tree_version.value().map(str::to_string);
    let filter_fp = file_filter.fingerprint();
    let filter = file_filter.into_filter();

    let source: Arc<dyn TreeSource> = match &tree_version {
        TreeVersion::Disk => Arc::new(DiskTreeSource::new(project.clone())),
        TreeVersion::Ref { r#ref } => Arc::new(GitTreeSource::new(project.clone(), r#ref.clone())),
        TreeVersion::Snapshot { .. } => {
            return Err(IpcError::invalid(
                "snapshot tree version is not yet implemented",
            ));
        }
    };

    let scan_id = state
        .code_quality_store
        .create_scan_with(
            "duplication",
            &scope,
            &kind_tag,
            value_str.as_deref(),
            &filter_fp,
        )
        .await?;
    state.events.emit(OxplowEvent::CodeQualityScanned {
        stream_id: None,
        scan_id,
        tool: "duplication".into(),
        scope: scope.clone(),
        phase: CodeQualityScanPhase::Started,
    });
    // Surface to the StatusBar's BackgroundTaskIndicator so the user
    // gets the standard "running" affordance while the scan runs.
    let bg_label = match &tree_version {
        TreeVersion::Disk => "Scanning duplicates (working tree)".to_string(),
        TreeVersion::Ref { r#ref } => {
            let short = if r#ref.len() > 12 {
                &r#ref[..7]
            } else {
                r#ref.as_str()
            };
            format!("Scanning duplicates @{short}")
        }
        TreeVersion::Snapshot { id } => format!("Scanning duplicates @snapshot {id}"),
    };
    let bg_task = state.background_tasks.start(StartInput {
        kind: BackgroundTaskKind::CodeQuality,
        label: bg_label,
        detail: Some(format!("scope: {scope}")),
        progress: None,
    });

    match run_duplication_scan_scoped(source, filter, None, None).await {
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
                tool: "duplication".into(),
                scope,
                phase: CodeQualityScanPhase::Completed,
            });
            state.background_tasks.complete(&bg_task.id, None);
            Ok(scan_id)
        }
        Err(e) => {
            state
                .code_quality_store
                .finish_scan(scan_id, CodeQualityScanStatus::Failed, Some(e.to_string()))
                .await?;
            state.events.emit(OxplowEvent::CodeQualityScanned {
                stream_id: None,
                scan_id,
                tool: "duplication".into(),
                scope,
                phase: CodeQualityScanPhase::Failed,
            });
            state
                .background_tasks
                .fail(&bg_task.id, e.to_string(), None);
            Err(IpcError::internal(e.to_string()))
        }
    }
}

/// Look up the most recent successful scan for `(tool, treeVersion,
/// fileFilter)`. The renderer uses this to decide whether to show
/// findings or a "Scan now" CTA.
#[tauri::command]
#[specta::specta]
pub async fn find_latest_code_quality_scan(
    state: tauri::State<'_, AppState>,
    tool: String,
    tree_version: TreeVersion,
    file_filter: FileFilterSpec,
) -> Result<Option<CodeQualityScan>, IpcError> {
    let kind_tag = tree_version.kind_tag().to_string();
    let value_str = tree_version.value().map(str::to_string);
    let filter_fp = file_filter.fingerprint();
    Ok(state
        .code_quality_store
        .find_latest_done_scan(&tool, &kind_tag, value_str.as_deref(), &filter_fp)
        .await?)
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
    /// Outer-to-inner names of the named-declaration ancestors this
    /// function lives inside (class / impl / module / namespace).
    /// Empty for top-level functions; used to render the Functions
    /// card hierarchically.
    pub container_path: Vec<String>,
    /// Heuristic public/private classification — see
    /// `oxplow_code_metrics::Visibility`. Frontend uses this to
    /// drive a "Show private" filter on the Semantic view.
    /// Serialized as `"public"` / `"private"` / `"unknown"`.
    pub visibility: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFileSide {
    pub path: String,
    /// `"base"` or `"head"`.
    pub side: String,
    pub functions: Vec<AnalyzedFunction>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFunctionChurn {
    pub name: String,
    pub container_path: Vec<String>,
    pub start_line_head: u32,
    pub added_lines: u32,
    pub deleted_lines: u32,
    pub modified_lines: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzedFileChurn {
    pub path: String,
    pub file_added: u32,
    pub file_deleted: u32,
    pub functions: Vec<AnalyzedFunctionChurn>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AnalyzeFunctionsResult {
    pub sides: Vec<AnalyzedFileSide>,
    /// One entry per file with both base + head content present —
    /// i.e. modified files. Added / deleted / unsupported / binary
    /// files are omitted (the file-level totals already cover those
    /// cases via `BranchChangeEntry.additions` / `deletions`).
    #[serde(default)]
    pub churn: Vec<AnalyzedFileChurn>,
}

/// Compute per-function metadata for the Change Analysis dashboard,
/// for both sides of the diff. Pure in-process call: walks each
/// (path, content) pair through tree-sitter.
#[tauri::command]
#[specta::specta]
pub async fn analyze_functions_at_refs(
    files: Vec<AnalyzeFileSpec>,
) -> Result<AnalyzeFunctionsResult, IpcError> {
    if files.is_empty() {
        return Ok(AnalyzeFunctionsResult {
            sides: Vec::new(),
            churn: Vec::new(),
        });
    }
    let result = tokio::task::spawn_blocking(move || analyze_files(files))
        .await
        .map_err(|e| IpcError::internal(format!("analyze task: {e}")))?;
    Ok(result)
}

fn analyze_files(files: Vec<AnalyzeFileSpec>) -> AnalyzeFunctionsResult {
    let mut sides: Vec<AnalyzedFileSide> = Vec::new();
    let mut churn: Vec<AnalyzedFileChurn> = Vec::new();
    for spec in files {
        // Run analyze_file once per side (working metrics for churn
        // attribution — we don't want to re-parse).
        let base_metrics = spec
            .base_content
            .as_deref()
            .map(|c| analyze_file(&spec.path, c))
            .unwrap_or_default();
        let head_metrics = spec
            .head_content
            .as_deref()
            .map(|c| analyze_file(&spec.path, c))
            .unwrap_or_default();

        if spec.base_content.is_some() {
            sides.push(AnalyzedFileSide {
                path: spec.path.clone(),
                side: "base".into(),
                functions: to_analyzed(base_metrics.clone()),
            });
        }
        if spec.head_content.is_some() {
            sides.push(AnalyzedFileSide {
                path: spec.path.clone(),
                side: "head".into(),
                functions: to_analyzed(head_metrics.clone()),
            });
        }

        if let (Some(base), Some(head)) =
            (spec.base_content.as_deref(), spec.head_content.as_deref())
        {
            let fc = crate::commands::churn::compute_file_churn(
                &spec.path,
                &base_metrics,
                &head_metrics,
                base,
                head,
            );
            churn.push(AnalyzedFileChurn {
                path: fc.path,
                file_added: fc.file_added,
                file_deleted: fc.file_deleted,
                functions: fc
                    .functions
                    .into_iter()
                    .map(|f| AnalyzedFunctionChurn {
                        name: f.name,
                        container_path: f.container_path,
                        start_line_head: f.start_line_head,
                        added_lines: f.added_lines,
                        deleted_lines: f.deleted_lines,
                        modified_lines: f.modified_lines,
                    })
                    .collect(),
            });
        }
    }
    sides.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.side.cmp(&b.side)));
    churn.sort_by(|a, b| a.path.cmp(&b.path));
    AnalyzeFunctionsResult { sides, churn }
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
            container_path: m.container_path,
            visibility: match m.visibility {
                Visibility::Public => "public",
                Visibility::Private => "private",
                Visibility::Unknown => "unknown",
            }
            .to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn analyze_functions_returns_function_for_each_side() {
        let files = vec![AnalyzeFileSpec {
            path: "src/foo.rs".into(),
            base_content: Some("fn a() {}".into()),
            head_content: Some("fn a() { if true { 1; } }".into()),
        }];
        let result = analyze_functions_at_refs(files).await.unwrap();
        assert_eq!(result.sides.len(), 2);
        let head = result.sides.iter().find(|s| s.side == "head").unwrap();
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

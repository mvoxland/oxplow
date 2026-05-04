import { useMemo, useState } from "react";
import type { ChangeAnalysisScope, ChangeAnalysisTarget } from "../../tabs/pageRefs.js";
import type { ChangeAnalysisState } from "./useChangeAnalysis.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { SummaryCard } from "./SummaryCard.js";
import { FunctionsCard } from "./FunctionsCard.js";
import { DuplicationCard } from "./DuplicationCard.js";
import { TestsCard } from "./TestsCard.js";
import { ChangeAnalysisFileTree } from "./FileTreeView.js";
import { buildFilePivots, summarizeTests, type FunctionsBuckets } from "./analysisHelpers.js";
import type { GitFileStatus } from "../../api-types.js";

type ViewMode = "semantic" | "files";
type StatusFilter = "all" | "added" | "modified" | "deleted";

export interface ChangeAnalysisDrilldownProps {
  scope: ChangeAnalysisScope;
  target: ChangeAnalysisTarget;
  analysis: ChangeAnalysisState;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
}

export function ChangeAnalysisDrilldown({
  scope,
  target,
  analysis,
  onOpenFile,
  onOpenDiff,
}: ChangeAnalysisDrilldownProps) {
  const [viewMode, setViewMode] = useState<ViewMode>("semantic");
  const initialStatus: StatusFilter = scope.kind === "status"
    ? (scope.value === "added" || scope.value === "modified" || scope.value === "deleted"
      ? scope.value
      : "all")
    : "all";
  const [statusFilter, setStatusFilter] = useState<StatusFilter>(initialStatus);
  void target;

  const filesAfterStatus = useMemo(() => {
    if (statusFilter === "all") return analysis.files;
    return analysis.files.filter((f) => matchesStatusFilter(f.status, statusFilter));
  }, [analysis.files, statusFilter]);

  const totalsAfterStatus = useMemo(() => {
    let add = 0;
    let del = 0;
    for (const f of filesAfterStatus) {
      add += f.additions ?? 0;
      del += f.deletions ?? 0;
    }
    return { additions: add, deletions: del };
  }, [filesAfterStatus]);

  const pivotsAfterStatus = useMemo(() => buildFilePivots(filesAfterStatus), [filesAfterStatus]);
  const testsAfterStatus = useMemo(() => summarizeTests(filesAfterStatus), [filesAfterStatus]);

  const filteredPathSet = useMemo(
    () => new Set(filesAfterStatus.map((f) => f.path)),
    [filesAfterStatus],
  );
  const functionsAfterStatus = useMemo<FunctionsBuckets>(() => ({
    added: analysis.functions.added.filter((fn) => filteredPathSet.has(fn.path)),
    deleted: analysis.functions.deleted.filter((fn) => filteredPathSet.has(fn.path)),
    modifiedSignature: analysis.functions.modifiedSignature.filter((fn) => filteredPathSet.has(fn.path)),
    modifiedBody: analysis.functions.modifiedBody.filter((fn) => filteredPathSet.has(fn.path)),
  }), [analysis.functions, filteredPathSet]);

  const dupAfterStatus = useMemo(() => ({
    ...analysis.duplication,
    findings: analysis.duplication.findings.filter((f) => filteredPathSet.has(f.path)),
  }), [analysis.duplication, filteredPathSet]);

  return (
    <>
      <SummaryCard
        fileCount={filesAfterStatus.length}
        additions={totalsAfterStatus.additions}
        deletions={totalsAfterStatus.deletions}
        byStatus={pivotsAfterStatus.byStatus}
        tests={testsAfterStatus}
      />

      <section style={card}>
        <div style={toolbarRow}>
          <div style={{ display: "flex", gap: 4 }}>
            {(["semantic", "files"] as ViewMode[]).map((m) => (
              <button
                key={m}
                type="button"
                data-testid={`change-analysis-view-${m}`}
                onClick={() => setViewMode(m)}
                style={viewMode === m ? activeTab : tab}
              >
                {m === "semantic" ? "Semantic" : "File list"}
              </button>
            ))}
          </div>
          <div style={{ display: "flex", gap: 4 }}>
            {(["all", "added", "modified", "deleted"] as StatusFilter[]).map((s) => (
              <button
                key={s}
                type="button"
                data-testid={`change-analysis-status-${s}`}
                onClick={() => setStatusFilter(s)}
                style={statusFilter === s ? activeTab : tab}
              >
                {s === "all" ? "All" : capitalize(s)}
              </button>
            ))}
          </div>
        </div>

        {filesAfterStatus.length === 0 ? (
          <div style={muted}>
            No files match the current status filter.
          </div>
        ) : viewMode === "semantic" ? (
          <FunctionsCard
            functions={functionsAfterStatus}
            target={target}
            onOpenFile={onOpenFile}
            onOpenFunctionDiff={onOpenDiff
              ? (path, line) => {
                if (!analysis.refs) {
                  onOpenFile(path);
                  return;
                }
                const { baseRef, headRef } = analysis.refs;
                const rightKind: DiffSpec["rightKind"] = headRef ? { ref: headRef } : "working";
                onOpenDiff({
                  path,
                  leftRef: baseRef,
                  rightKind,
                  baseLabel: target === "working" ? "working tree" : `parent of ${target.toString().slice(0, 7)}`,
                  revealLine: line,
                });
              }
              : undefined}
          />
        ) : (
          <ChangeAnalysisFileTree
            files={filesAfterStatus}
            onOpenFile={onOpenFile}
          />
        )}
      </section>

      <DuplicationCard duplication={dupAfterStatus} onOpenFile={onOpenFile} />
      <TestsCard tests={testsAfterStatus} onOpenFile={onOpenFile} />
    </>
  );
}

function matchesStatusFilter(status: GitFileStatus, filter: StatusFilter): boolean {
  if (filter === "added") return status === "added" || status === "untracked";
  if (filter === "modified") return status === "modified" || status === "renamed";
  if (filter === "deleted") return status === "deleted";
  return true;
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const toolbarRow: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
  marginBottom: 12,
  flexWrap: "wrap",
  gap: 8,
};
const tab: React.CSSProperties = {
  padding: "4px 10px",
  background: "transparent",
  color: "var(--text-muted)",
  borderWidth: 1,
  borderStyle: "solid",
  borderColor: "var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 12,
};
const activeTab: React.CSSProperties = {
  ...tab,
  background: "var(--accent-soft-bg, var(--surface-app))",
  color: "var(--text-primary)",
  borderColor: "var(--text-link, #2563eb)",
};
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13 };

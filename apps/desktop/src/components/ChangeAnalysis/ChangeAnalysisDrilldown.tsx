import { useMemo, useState } from "react";
import type { ChangeAnalysisScope, ChangeAnalysisTarget } from "../../tabs/pageRefs.js";
import type { ChangeAnalysisState } from "./useChangeAnalysis.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { DISK, refVersion, type FileVersion } from "../../file-version.js";
import { SummaryCard } from "./SummaryCard.js";
import { FunctionsCard } from "./FunctionsCard.js";
import { DuplicationCard } from "./DuplicationCard.js";
import { TestsCard } from "./TestsCard.js";
import { ChangeAnalysisFileTree } from "./FileTreeView.js";
import { LookHereFirstCard } from "./LookHereFirstCard.js";
import { FileChurnCard } from "./FileChurnCard.js";
import { FunctionChurnCard } from "./FunctionChurnCard.js";
import { ComplexitySpikesCard } from "./ComplexitySpikesCard.js";
import { OtherSmellsCard } from "./OtherSmellsCard.js";
import { FilesPivot } from "./FilesPivot.js";
import { buildFilePivots, summarizeTests, type FunctionChurnRow, type FunctionsBuckets } from "./analysisHelpers.js";
import type { GitFileStatus } from "../../api-types.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";

type ViewMode = "semantic" | "files";
type StatusFilter = "all" | "added" | "modified" | "deleted";

export interface ChangeAnalysisDrilldownProps {
  /** Optional drilldown filter; when absent, every panel still
   *  renders against the full file set. */
  scope?: ChangeAnalysisScope;
  target: ChangeAnalysisTarget;
  analysis: ChangeAnalysisState;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec): void;
}

export function ChangeAnalysisDrilldown({
  scope,
  target,
  analysis,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: ChangeAnalysisDrilldownProps) {
  const [viewMode, setViewMode] = useState<ViewMode>("semantic");
  const initialStatus: StatusFilter = scope?.kind === "status"
    ? (scope.value === "added" || scope.value === "modified" || scope.value === "deleted"
      ? scope.value
      : "all")
    : "all";
  const [statusFilter, setStatusFilter] = useState<StatusFilter>(initialStatus);
  // Persist the view-toggle + status-filter selections across restart
  // so the user lands on the same configuration they left.
  usePageSnapshot<{ viewMode: ViewMode; statusFilter: StatusFilter }>({
    serialize: () => ({ viewMode, statusFilter }),
    restore: (snap) => {
      if (snap.viewMode === "semantic" || snap.viewMode === "files") {
        setViewMode(snap.viewMode);
      }
      if (
        snap.statusFilter === "all" || snap.statusFilter === "added" ||
        snap.statusFilter === "modified" || snap.statusFilter === "deleted"
      ) {
        setStatusFilter(snap.statusFilter);
      }
    },
    deps: [viewMode, statusFilter],
  });
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

  const churnAfterStatus = useMemo<FunctionChurnRow[]>(
    () => analysis.functionChurn.filter((c) => filteredPathSet.has(c.path)),
    [analysis.functionChurn, filteredPathSet],
  );

  /**
   * Open the diff for `path` at `line` *in the current tab*. The
   * preferred path is `onOpenDiffInTab` — that swaps the tab's ref
   * with the diff so back returns to this analysis page. Falls
   * through to `onOpenDiff` (separate tab) if the host didn't
   * supply the in-tab variant, and to plain file-open if neither
   * the diff handler nor refs are available.
   */
  const openDiffAt = (path: string, line: number) => {
    if (!analysis.refs) {
      onOpenFile(path);
      return;
    }
    const { baseRef, headRef } = analysis.refs;
    const rightVersion: FileVersion = headRef ? refVersion(headRef) : DISK;
    const spec: DiffSpec = {
      path,
      leftVersion: refVersion(baseRef),
      rightVersion,
      baseLabel: target === "working" ? "working tree" : `parent of ${target.toString().slice(0, 7)}`,
      revealLine: line,
    };
    if (onOpenDiffInTab) onOpenDiffInTab(spec);
    else if (onOpenDiff) onOpenDiff(spec);
    else onOpenFile(path);
  };

  // The duplication card needs to know which tree version the scan
  // ran against so it can stamp every duplicate-block ref with that
  // version (the side-by-side viewer reads file content at this
  // version). For working-tree analysis it's `disk`; for a commit it
  // matches the analyzed commit's tree.
  const scanVersion: FileVersion = target === "working" ? DISK : refVersion(target);

  return (
    <>
      <LookHereFirstCard
        files={filesAfterStatus}
        fileScores={analysis.fileScores}
        onOpenFile={onOpenFile}
      />
      <SummaryCard
        fileCount={filesAfterStatus.length}
        additions={totalsAfterStatus.additions}
        deletions={totalsAfterStatus.deletions}
        byStatus={pivotsAfterStatus.byStatus}
        tests={testsAfterStatus}
      />
      <FileChurnCard files={filesAfterStatus} onOpenFile={onOpenFile} />
      <FilesPivot pivots={pivotsAfterStatus} target={target} />

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
            onOpenFunctionDiff={openDiffAt}
          />
        ) : (
          <ChangeAnalysisFileTree
            files={filesAfterStatus}
            onOpenFile={onOpenFile}
            onOpenFileDiff={(path) => openDiffAt(path, 1)}
          />
        )}
      </section>

      <FunctionChurnCard
        churn={churnAfterStatus}
        functions={functionsAfterStatus}
        onOpenFile={onOpenFile}
      />
      <ComplexitySpikesCard functions={functionsAfterStatus} onOpenFile={onOpenFile} />
      <OtherSmellsCard
        functions={functionsAfterStatus}
        tests={testsAfterStatus}
        onOpenFile={onOpenFile}
      />
      <DuplicationCard duplication={dupAfterStatus} scanVersion={scanVersion} onOpenFile={onOpenFile} />
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

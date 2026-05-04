import { useMemo } from "react";
import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, uncommittedChangesRef, type ChangeAnalysisScope } from "../tabs/pageRefs.js";
import { useChangeAnalysis } from "../components/ChangeAnalysis/useChangeAnalysis.js";
import { SummaryCard } from "../components/ChangeAnalysis/SummaryCard.js";
import { FilesPivot } from "../components/ChangeAnalysis/FilesPivot.js";
import { ChangeAnalysisHeader } from "../components/ChangeAnalysis/ChangeAnalysisHeader.js";
import { ChangeAnalysisDrilldown } from "../components/ChangeAnalysis/ChangeAnalysisDrilldown.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";

export interface ChangeAnalysisPageProps {
  stream: Stream | null;
  target: string;
  /** Optional drilldown scope. When present, the page renders the
   *  focused layout (semantic / file-list toggle, status filter,
   *  duplication + tests cards). Absent → dashboard layout. */
  scope?: ChangeAnalysisScope;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** Open the diff for a path between the analysis's resolved base
   *  and head refs. The page builds the spec; the host wires it to
   *  the existing diff-tab opener. */
  onOpenDiff?(spec: DiffSpec): void;
  /** Replace the current change-analysis tab with a diff view in
   *  place — browser-tab semantic, back returns to the analysis
   *  dashboard. Preferred over `onOpenDiff` for in-page clicks. */
  onOpenDiffInTab?(spec: DiffSpec): void;
}

export function ChangeAnalysisPage({ stream, target, scope, onOpenPage, onOpenFile, onOpenDiff, onOpenDiffInTab }: ChangeAnalysisPageProps) {
  const streamId = stream?.id ?? null;
  const analysis = useChangeAnalysis({ streamId, target, scope });

  const headerLabel = useMemo(() => {
    const base = target === "working" ? "Uncommitted Changes" : `Commit ${target.slice(0, 7)}`;
    if (!scope) return base;
    return `${base} — ${formatScope(scope)}`;
  }, [target, scope]);

  const sourceLink = useMemo<TabRef | null>(() => {
    if (target === "working") return uncommittedChangesRef();
    return gitCommitRef(target);
  }, [target]);

  if (!streamId) {
    return (
      <Page testId="page-change-analysis" title={`Change Analysis: ${headerLabel}`}>
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-change-analysis" title={`Change Analysis: ${headerLabel}`}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        <ChangeAnalysisHeader
          target={target}
          loading={analysis.loading}
          onRefresh={() => void analysis.refresh()}
          sourceLink={sourceLink}
          onOpenPage={onOpenPage}
        />

        {analysis.error ? <div style={errorBanner}>{analysis.error}</div> : null}

        {analysis.loading && analysis.files.length === 0 ? (
          <div style={muted}>Loading…</div>
        ) : analysis.files.length === 0 ? (
          <div style={muted}>
            {scope
              ? `No files match ${formatScope(scope)}.`
              : target === "working"
                ? "Working tree is clean."
                : "No file changes in this commit."}
          </div>
        ) : scope ? (
          <ChangeAnalysisDrilldown
            scope={scope}
            target={target}
            analysis={analysis}
            onOpenFile={onOpenFile}
            onOpenDiff={onOpenDiff}
            onOpenDiffInTab={onOpenDiffInTab}
          />
        ) : (
          <>
            <SummaryCard
              fileCount={analysis.files.length}
              additions={analysis.totals.additions}
              deletions={analysis.totals.deletions}
              byStatus={analysis.pivots.byStatus}
              tests={analysis.tests}
            />
            <FilesPivot pivots={analysis.pivots} target={target} />
          </>
        )}
      </div>
    </Page>
  );
}

function formatScope(scope: ChangeAnalysisScope): string {
  if (scope.kind === "ext") return `.${scope.value} files`;
  if (scope.kind === "dir") return `${scope.value}/`;
  return scope.value;
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13, padding: 16 };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};

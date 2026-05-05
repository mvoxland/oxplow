import { useMemo } from "react";
import { gitCommitRef, uncommittedChangesRef, type ChangeAnalysisScope } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { useChangeAnalysis } from "./useChangeAnalysis.js";
import { ChangeAnalysisHeader } from "./ChangeAnalysisHeader.js";
import { ChangeAnalysisDrilldown } from "./ChangeAnalysisDrilldown.js";

export interface ChangeAnalysisPanelProps {
  /** Stream id; renders an empty placeholder if null. */
  streamId: string | null;
  /** "working" or a commit SHA. */
  target: string;
  /** Optional drilldown filter; data is filtered by scope but every
   *  panel still renders. */
  scope?: ChangeAnalysisScope;
  /** Whether to render the ChangeAnalysisHeader (back-to-source +
   *  refresh). Hosts that already have their own header (e.g. the
   *  GitCommitPage) hide it. */
  showHeader?: boolean;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** Open the diff for a path between the analysis's resolved base
   *  and head refs. The panel builds the spec; the host wires it. */
  onOpenDiff?(spec: DiffSpec): void;
  /** Replace the host tab with the diff in place — preferred. */
  onOpenDiffInTab?(spec: DiffSpec): void;
}

/**
 * Reusable Change Analysis content. Renders the optional header,
 * the error banner, an empty / loading placeholder, or the full
 * drilldown panel set. Stripped of any `Page` chrome so other
 * surfaces (the GitCommitPage in particular) can host it inline
 * below their own content.
 *
 * `ChangeAnalysisPage` is the thin wrapper that surrounds this
 * with a `Page` for the standalone "Analysis: <commit>" tab.
 */
export function ChangeAnalysisPanel({
  streamId,
  target,
  scope,
  showHeader = true,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: ChangeAnalysisPanelProps) {
  const analysis = useChangeAnalysis({ streamId, target, scope });

  const sourceLink = useMemo<TabRef | null>(() => {
    if (target === "working") return uncommittedChangesRef();
    return gitCommitRef(target);
  }, [target]);

  if (!streamId) {
    return <div style={muted}>No stream selected.</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      {showHeader ? (
        <ChangeAnalysisHeader
          target={target}
          loading={analysis.loading}
          onRefresh={() => void analysis.refresh()}
          sourceLink={sourceLink}
          onOpenPage={onOpenPage}
        />
      ) : null}

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
      ) : (
        <ChangeAnalysisDrilldown
          scope={scope}
          target={target}
          analysis={analysis}
          onOpenFile={onOpenFile}
          onOpenDiff={onOpenDiff}
          onOpenDiffInTab={onOpenDiffInTab}
        />
      )}
    </div>
  );
}

export function formatChangeAnalysisScope(scope: ChangeAnalysisScope): string {
  return formatScope(scope);
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

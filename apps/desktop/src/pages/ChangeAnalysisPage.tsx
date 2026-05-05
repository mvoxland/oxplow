import { useMemo } from "react";
import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import type { ChangeAnalysisScope } from "../tabs/pageRefs.js";
import {
  ChangeAnalysisPanel,
  formatChangeAnalysisScope,
} from "../components/ChangeAnalysis/ChangeAnalysisPanel.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";

export interface ChangeAnalysisPageProps {
  stream: Stream | null;
  target: string;
  /** Optional drilldown scope. */
  scope?: ChangeAnalysisScope;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec): void;
}

/**
 * Standalone Change Analysis tab. Thin Page chrome around
 * ChangeAnalysisPanel — kept so other entry points (the explicit
 * "Analyze Changes" link, deep-linking by commit ref) still resolve
 * to a top-level page. Other surfaces (the GitCommitPage) embed
 * the panel directly.
 */
export function ChangeAnalysisPage({ stream, target, scope, onOpenPage, onOpenFile, onOpenDiff, onOpenDiffInTab }: ChangeAnalysisPageProps) {
  const headerLabel = useMemo(() => {
    const base = target === "working" ? "Uncommitted Changes" : `Commit ${target.slice(0, 7)}`;
    if (!scope) return base;
    return `${base} — ${formatChangeAnalysisScope(scope)}`;
  }, [target, scope]);

  return (
    <Page testId="page-change-analysis" title={`Change Analysis: ${headerLabel}`}>
      <div style={{ padding: 16, overflow: "auto" }}>
        <ChangeAnalysisPanel
          streamId={stream?.id ?? null}
          target={target}
          scope={scope}
          onOpenPage={onOpenPage}
          onOpenFile={onOpenFile}
          onOpenDiff={onOpenDiff}
          onOpenDiffInTab={onOpenDiffInTab}
        />
      </div>
    </Page>
  );
}

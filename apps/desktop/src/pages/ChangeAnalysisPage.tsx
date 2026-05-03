import { useMemo } from "react";
import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, uncommittedChangesRef } from "../tabs/pageRefs.js";
import { useChangeAnalysis } from "../components/ChangeAnalysis/useChangeAnalysis.js";
import { SummaryCard } from "../components/ChangeAnalysis/SummaryCard.js";
import { FilesPivot } from "../components/ChangeAnalysis/FilesPivot.js";
import { FunctionsCard } from "../components/ChangeAnalysis/FunctionsCard.js";
import { DuplicationCard } from "../components/ChangeAnalysis/DuplicationCard.js";
import { TestsCard } from "../components/ChangeAnalysis/TestsCard.js";

export interface ChangeAnalysisPageProps {
  stream: Stream | null;
  target: string;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

export function ChangeAnalysisPage({ stream, target, onOpenPage, onOpenFile }: ChangeAnalysisPageProps) {
  const streamId = stream?.id ?? null;
  const analysis = useChangeAnalysis({ streamId, target });

  const headerLabel = useMemo(() => {
    if (target === "working") return "Uncommitted Changes";
    return `Commit ${target.slice(0, 7)}`;
  }, [target]);

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
        <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
          <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
            {target === "working" ? "Working tree (HEAD vs uncommitted)" : `Parent vs ${target.slice(0, 12)}`}
          </span>
          <button
            type="button"
            data-testid="change-analysis-refresh"
            onClick={() => void analysis.refresh()}
            disabled={analysis.loading}
            style={smallButton}
          >
            {analysis.loading ? "Loading…" : "Refresh"}
          </button>
          {sourceLink ? (
            <button
              type="button"
              data-testid="change-analysis-open-source"
              onClick={() => onOpenPage(sourceLink)}
              style={linkButton}
            >
              {target === "working" ? "Open Uncommitted →" : "Open Commit →"}
            </button>
          ) : null}
        </div>

        {analysis.error ? <div style={errorBanner}>{analysis.error}</div> : null}

        {analysis.loading && analysis.files.length === 0 ? (
          <div style={muted}>Loading…</div>
        ) : analysis.files.length === 0 ? (
          <div style={muted}>
            {target === "working" ? "Working tree is clean." : "No file changes in this commit."}
          </div>
        ) : (
          <>
            <SummaryCard
              fileCount={analysis.files.length}
              additions={analysis.totals.additions}
              deletions={analysis.totals.deletions}
              byStatus={analysis.pivots.byStatus}
              tests={analysis.tests}
            />
            <FilesPivot pivots={analysis.pivots} files={analysis.files} onOpenFile={onOpenFile} />
            <FunctionsCard functions={analysis.functions} onOpenFile={onOpenFile} />
            <DuplicationCard
              duplication={analysis.duplication}
              onOpenFile={onOpenFile}
            />
            <TestsCard tests={analysis.tests} onOpenFile={onOpenFile} />
          </>
        )}
      </div>
    </Page>
  );
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13, padding: 16 };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
const smallButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 12,
};
const linkButton: React.CSSProperties = {
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  fontSize: 12,
  cursor: "pointer",
};

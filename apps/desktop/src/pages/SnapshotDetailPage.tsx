import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Snapshot, Stream } from "../api.js";
import { listSnapshots } from "../api.js";
import { logUi } from "../logger.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, snapshotRef } from "../tabs/pageRefs.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { ChangeAnalysisPanel } from "../components/ChangeAnalysis/ChangeAnalysisPanel.js";
import { SummaryCard } from "../components/ChangeAnalysis/SummaryCard.js";
import { useChangeAnalysis } from "../components/ChangeAnalysis/useChangeAnalysis.js";
import {
  summarizeTestFunctions,
  summarizeTestLineRatio,
} from "../components/ChangeAnalysis/analysisHelpers.js";
import { formatFullDateTime } from "../components/format.js";

export interface SnapshotDetailPageProps {
  stream: Stream | null;
  snapshotId: number;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile?(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Single snapshot page — drilled into from the Local History
 * dashboard's Recent Snapshots card. Mirrors GitCommitPage: a
 * SummaryCard on the right, snapshot metadata on the left, and the
 * shared ChangeAnalysisPanel below for the file list + function /
 * churn / interestingness breakdowns.
 *
 * The change-analysis pipeline is fed via a `target` string of
 * `"snapshot:<id>"`; useChangeAnalysis recognizes that and routes
 * file lookups + content reads through the snapshot store instead
 * of git refs.
 */
export function SnapshotDetailPage({
  stream,
  snapshotId,
  onOpenDiff,
  onOpenDiffInTab,
  onOpenPage,
  onOpenFile,
}: SnapshotDetailPageProps) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [loadingSnapshot, setLoadingSnapshot] = useState(false);
  const refForGraph = snapshotRef(snapshotId);
  const backlinkEntries = useBacklinks(refForGraph);
  const outboundEntries = usePageOutbound(refForGraph);
  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />,
  };
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} onOpenPage={onOpenPage} />,
        }
      : undefined;
  const target = `snapshot:${snapshotId}`;
  const analysis = useChangeAnalysis({ streamId: stream?.id ?? null, target });

  // Measure the SummaryCard so the SnapshotMeta on its left collapses
  // to the same height — same pattern GitCommitPage uses.
  const summaryRef = useRef<HTMLDivElement>(null);
  const [summaryHeight, setSummaryHeight] = useState<number | null>(null);
  useLayoutEffect(() => {
    const el = summaryRef.current;
    if (!el) {
      setSummaryHeight(null);
      return;
    }
    const measure = () => setSummaryHeight(el.getBoundingClientRect().height);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [analysis.files.length, snapshotId, stream?.id]);

  useEffect(() => {
    if (!stream) {
      setSnapshot(null);
      return;
    }
    let cancelled = false;
    setLoadingSnapshot(true);
    // No "get one snapshot" IPC; pull the recent window and pick our id.
    // Cheap (~500 rows) — acceptable for a page-load fetch.
    void listSnapshots(stream.id, 500)
      .then((rows) => {
        if (cancelled) return;
        setSnapshot(rows.find((r) => r.id === snapshotId) ?? null);
        setLoadingSnapshot(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot fetch failed", { error: String(err) });
        setLoadingSnapshot(false);
      });
    return () => {
      cancelled = true;
    };
  }, [stream?.id, snapshotId]);

  const headerTitle = useMemo(() => {
    if (!snapshot) return `Snapshot ${snapshotId}`;
    return `Snapshot ${snapshotId} · ${formatFullDateTime(snapshot.createdAt)}`;
  }, [snapshot, snapshotId]);

  return (
    <Page testId="page-snapshot-detail" title={headerTitle} kind="snapshot" backlinks={backlinks} outbound={outbound}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: "12px 16px" }}>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(0, 3fr) minmax(0, 2fr)",
            gap: 16,
            alignItems: "start",
          }}
        >
          <div style={{ minWidth: 0 }}>
            {loadingSnapshot && !snapshot ? (
              <div style={muted}>Loading…</div>
            ) : !snapshot ? (
              <div style={muted}>Snapshot not found.</div>
            ) : (
              <SnapshotMeta
                snapshot={snapshot}
                collapsedMaxHeight={summaryHeight}
                onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))}
              />
            )}
          </div>
          <div style={{ minWidth: 0 }} ref={summaryRef}>
            {snapshot && stream && analysis.files.length > 0 ? (
              <SummaryCard
                fileCount={analysis.files.length}
                additions={analysis.totals.additions}
                deletions={analysis.totals.deletions}
                byStatus={analysis.pivots.byStatus}
                tests={analysis.tests}
                testFunctions={summarizeTestFunctions(analysis.functions)}
                testLineRatio={summarizeTestLineRatio(analysis.functionChurn)}
              />
            ) : null}
          </div>
        </div>

        {snapshot && stream && onOpenFile ? (
          <ChangeAnalysisPanel
            analysis={analysis}
            target={target}
            showHeader={false}
            onOpenPage={onOpenPage}
            onOpenFile={onOpenFile}
            onOpenDiff={onOpenDiff}
            onOpenDiffInTab={onOpenDiffInTab}
          />
        ) : null}
      </div>
    </Page>
  );
}

interface SnapshotMetaProps {
  snapshot: Snapshot;
  /** Pixel height the SummaryCard alongside this panel renders at —
   *  the panel collapses to roughly that height. `null` lets it grow. */
  collapsedMaxHeight: number | null;
  onOpenCommit(sha: string): void;
}

function SnapshotMeta({ snapshot, collapsedMaxHeight, onOpenCommit }: SnapshotMetaProps) {
  const captured = formatFullDateTime(snapshot.createdAt);
  return (
    <section
      style={{
        ...card,
        maxHeight: collapsedMaxHeight ?? undefined,
        overflow: collapsedMaxHeight ? "hidden" : undefined,
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--text-xs)" }}>
        <div style={{ color: "var(--text-secondary)", fontSize: 11 }}>
          Captured {captured}
        </div>
        <div>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>
            Snapshot {snapshot.id}
          </div>
          <div style={{ color: "var(--text-secondary)", fontSize: 11 }}>
            {snapshot.fileCount} files captured in this snapshot.
          </div>
        </div>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "auto 1fr",
            gap: "2px 10px",
            color: "var(--text-secondary)",
            fontSize: 11,
          }}
        >
          <span>Git commit</span>
          <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--text-primary)" }}>
            {snapshot.gitCommit ? (
              <button
                type="button"
                onClick={() => onOpenCommit(snapshot.gitCommit!)}
                style={linkButton}
              >
                {snapshot.gitCommit.slice(0, 7)}
              </button>
            ) : (
              <span style={{ color: "var(--text-secondary)" }}>—</span>
            )}
          </span>
        </div>
      </div>
    </section>
  );
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)" };
const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  position: "relative",
};
const linkButton: React.CSSProperties = {
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  fontFamily: "var(--mono, monospace)",
  fontSize: "var(--text-xs)",
  cursor: "pointer",
};

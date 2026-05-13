import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ParentSnapshot, Stream } from "../api.js";
import { listParentSnapshots } from "../api.js";
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
 * Single parent-snapshot page — drilled into from the Local History
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
  const [parent, setParent] = useState<ParentSnapshot | null>(null);
  const [loadingParent, setLoadingParent] = useState(false);
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
      setParent(null);
      return;
    }
    let cancelled = false;
    setLoadingParent(true);
    // No "get one parent" IPC; pull the recent window and pick our id.
    // Cheap (~500 rows) — acceptable for a page-load fetch.
    void listParentSnapshots(stream.id, 500)
      .then((rows) => {
        if (cancelled) return;
        setParent(rows.find((r) => r.id === snapshotId) ?? null);
        setLoadingParent(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot parent fetch failed", { error: String(err) });
        setLoadingParent(false);
      });
    return () => {
      cancelled = true;
    };
  }, [stream?.id, snapshotId]);

  const headerTitle = useMemo(() => {
    if (!parent) return `Snapshot ${snapshotId}`;
    return `Snapshot ${snapshotId} · ${formatFullDateTime(parent.createdAt)}`;
  }, [parent, snapshotId]);

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
            {loadingParent && !parent ? (
              <div style={muted}>Loading…</div>
            ) : !parent ? (
              <div style={muted}>Snapshot not found.</div>
            ) : (
              <SnapshotMeta
                parent={parent}
                collapsedMaxHeight={summaryHeight}
                onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))}
              />
            )}
          </div>
          <div style={{ minWidth: 0 }} ref={summaryRef}>
            {parent && stream && analysis.files.length > 0 ? (
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

        {parent && stream && onOpenFile ? (
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
  parent: ParentSnapshot;
  /** Pixel height the SummaryCard alongside this panel renders at —
   *  the panel collapses to roughly that height. `null` lets it grow. */
  collapsedMaxHeight: number | null;
  onOpenCommit(sha: string): void;
}

function SnapshotMeta({ parent, collapsedMaxHeight, onOpenCommit }: SnapshotMetaProps) {
  const captured = formatFullDateTime(parent.createdAt);
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
            Snapshot {parent.id}
          </div>
          <div style={{ color: "var(--text-secondary)", fontSize: 11 }}>
            {parent.fileCount} files captured under this parent.
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
          <span>Pinned commit</span>
          <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--text-primary)" }}>
            {parent.gitCommit ? (
              <button
                type="button"
                onClick={() => onOpenCommit(parent.gitCommit!)}
                style={linkButton}
              >
                {parent.gitCommit.slice(0, 7)}
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

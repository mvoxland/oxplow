import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { EffortAtSnapshot, Snapshot, Stream } from "../api.js";
import {
  getTaskSummaries,
  listChangedPathsForEffort,
  listEffortFiles,
  listEffortsAtSnapshots,
  listSnapshots,
} from "../api.js";
import { logUi } from "../logger.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, snapshotRef, taskRef } from "../tabs/pageRefs.js";
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
interface EffortRow {
  effort: EffortAtSnapshot;
  taskTitle: string;
  files: Array<{ path: string; change: "created" | "updated" | "deleted" }>;
  /** Auto-diff between start_snapshot_id and end_snapshot_id —
   *  every path that changed during the effort window regardless of
   *  whether the agent claimed it. Reference list shown alongside
   *  the canonical `files` so the user can spot omissions. Empty
   *  for in-flight efforts (end snapshot id isn't pinned yet). */
  autoDiff: string[];
}

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
  const [completedEfforts, setCompletedEfforts] = useState<EffortRow[]>([]);
  const [inFlightEfforts, setInFlightEfforts] = useState<EffortRow[]>([]);
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

  // Efforts active at this snapshot, partitioned into "completed
  // here" (end_snapshot_id == this snapshot — the "this is what
  // shipped" view) and "in flight" (start_snapshot_id <= this AND
  // end is later or NULL). Each row carries the canonical
  // task_effort_file list — the LLM-declared authorship (after
  // any amend_effort), not the raw snapshot diff. The auto-diff
  // is only fetched for completed efforts (it requires both start
  // and end snapshot ids).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const all = await listEffortsAtSnapshots([snapshotId]);
        if (all.length === 0) {
          if (!cancelled) {
            setCompletedEfforts([]);
            setInFlightEfforts([]);
          }
          return;
        }
        const completed = all.filter((e) => e.completedHere);
        const inFlight = all.filter((e) => !e.completedHere);
        const titles = await getTaskSummaries(
          Array.from(new Set(all.map((e) => e.tasksId))),
        ).catch(() => [] as Array<{ id: number; title: string }>);
        const titleByTask = new Map<number, string>(
          titles.map((t) => [t.id, t.title] as [number, string]),
        );
        type EffortFileRow = { path: string; change: "created" | "updated" | "deleted" };
        const filesByEffort: Array<[string, EffortFileRow[]]> = await Promise.all(
          all.map(async (e) => {
            try {
              const files = await listEffortFiles(e.effortId);
              return [e.effortId, files] as [string, EffortFileRow[]];
            } catch (err) {
              logUi("warn", "effort files fetch failed", {
                error: String(err),
                effortId: e.effortId,
              });
              return [e.effortId, [] as EffortFileRow[]] as [string, EffortFileRow[]];
            }
          }),
        );
        const filesById = new Map<string, EffortFileRow[]>(filesByEffort);
        const diffsByEffort: Array<[string, string[]]> = await Promise.all(
          completed.map(async (e) => {
            try {
              const paths = await listChangedPathsForEffort(e.effortId);
              return [e.effortId, paths] as [string, string[]];
            } catch (err) {
              logUi("warn", "effort auto-diff fetch failed", {
                error: String(err),
                effortId: e.effortId,
              });
              return [e.effortId, [] as string[]] as [string, string[]];
            }
          }),
        );
        const diffById = new Map<string, string[]>(diffsByEffort);
        if (cancelled) return;
        const toRow = (effort: EffortAtSnapshot): EffortRow => ({
          effort,
          taskTitle: titleByTask.get(effort.tasksId) ?? `task ${effort.tasksId}`,
          files: filesById.get(effort.effortId) ?? [],
          autoDiff: diffById.get(effort.effortId) ?? [],
        });
        setCompletedEfforts(completed.map(toRow));
        setInFlightEfforts(inFlight.map(toRow));
      } catch (err) {
        logUi("warn", "efforts at snapshot fetch failed", { error: String(err) });
        if (!cancelled) {
          setCompletedEfforts([]);
          setInFlightEfforts([]);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [snapshotId]);

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

        {completedEfforts.length > 0 ? (
          <EffortsSection
            title="Efforts completed at this snapshot"
            rows={completedEfforts}
            showAutoDiff
            onOpenTask={(taskId) => onOpenPage(taskRef(taskId))}
            onOpenSnapshot={(id) => onOpenPage(snapshotRef(id))}
            onOpenFile={onOpenFile ? (path) => onOpenFile(path) : undefined}
          />
        ) : null}

        {inFlightEfforts.length > 0 ? (
          <EffortsSection
            title="Efforts in progress at this snapshot"
            rows={inFlightEfforts}
            showAutoDiff={false}
            onOpenTask={(taskId) => onOpenPage(taskRef(taskId))}
            onOpenSnapshot={(id) => onOpenPage(snapshotRef(id))}
            onOpenFile={onOpenFile ? (path) => onOpenFile(path) : undefined}
          />
        ) : null}

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

function EffortsSection({
  title,
  rows,
  showAutoDiff,
  onOpenTask,
  onOpenSnapshot,
  onOpenFile,
}: {
  title: string;
  rows: EffortRow[];
  /** Show the "Also changed in window" reference list per row.
   *  Only meaningful for completed efforts (in-flight ones have no
   *  end snapshot pinned yet). */
  showAutoDiff: boolean;
  onOpenTask(taskId: number): void;
  onOpenSnapshot(snapshotId: number): void;
  onOpenFile?: (path: string) => void;
}) {
  return (
    <section style={card}>
      <div style={{ fontWeight: 600, marginBottom: 8, fontSize: "var(--text-sm)" }}>
        {title}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        {rows.map(({ effort, taskTitle, files, autoDiff }) => {
          const claimedSet = new Set(files.map((f) => f.path));
          const referenceOnly = showAutoDiff
            ? autoDiff.filter((p) => !claimedSet.has(p))
            : [];
          return (
            <div
              key={effort.effortId}
              style={{ display: "flex", flexDirection: "column", gap: 4 }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <button
                  type="button"
                  onClick={() => onOpenTask(effort.tasksId)}
                  style={{ ...linkButton, fontFamily: "inherit", fontWeight: 600 }}
                >
                  {taskTitle}
                </button>
                {effort.startSnapshotId !== null ? (
                  <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>
                    · started at{" "}
                    <button
                      type="button"
                      onClick={() => onOpenSnapshot(effort.startSnapshotId!)}
                      style={linkButton}
                    >
                      snapshot {effort.startSnapshotId}
                    </button>
                  </span>
                ) : null}
              </div>
              {files.length === 0 && referenceOnly.length === 0 ? (
                <div style={{ color: "var(--text-muted)", fontSize: 11, paddingLeft: 4 }}>
                  {showAutoDiff
                    ? "No files declared and no diff in the effort window."
                    : "No files declared yet for this effort."}
                </div>
              ) : null}
              {files.length > 0 ? (
                <div>
                  <div
                    style={{
                      color: "var(--text-muted)",
                      fontSize: 10,
                      paddingLeft: 4,
                      marginBottom: 2,
                    }}
                  >
                    Claimed (canonical authorship)
                  </div>
                  <ul style={listStyle}>
                    {files.map((f) => (
                      <li key={f.path}>
                        {onOpenFile ? (
                          <button
                            type="button"
                            onClick={() => onOpenFile(f.path)}
                            style={{ ...linkButton, fontFamily: "var(--mono, monospace)" }}
                            title={`${f.change}: ${f.path}`}
                          >
                            {f.path}
                          </button>
                        ) : (
                          <span style={{ fontFamily: "var(--mono, monospace)" }}>{f.path}</span>
                        )}
                        <span style={{ marginLeft: 6, color: "var(--text-muted)" }}>
                          {f.change}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              {referenceOnly.length > 0 ? (
                <div>
                  <div
                    style={{
                      color: "var(--text-muted)",
                      fontSize: 10,
                      paddingLeft: 4,
                      marginBottom: 2,
                    }}
                    title="All paths whose file_snapshot rows fall in (start_snapshot, end_snapshot] but the agent didn't claim. Could be parallel efforts, formatters, or omissions."
                  >
                    Also changed in window (reference, not claimed)
                  </div>
                  <ul style={referenceListStyle}>
                    {referenceOnly.map((path) => (
                      <li key={path}>
                        {onOpenFile ? (
                          <button
                            type="button"
                            onClick={() => onOpenFile(path)}
                            style={{ ...linkButton, fontFamily: "var(--mono, monospace)" }}
                          >
                            {path}
                          </button>
                        ) : (
                          <span style={{ fontFamily: "var(--mono, monospace)" }}>{path}</span>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
      <div style={{ color: "var(--text-muted)", fontSize: 10, marginTop: 8 }}>
        {showAutoDiff ? (
          <>
            "Claimed" is the agent's declared authorship via{" "}
            <code>complete_task</code>/<code>amend_effort</code>.
            "Also changed in window" is the auto-diff between the
            effort's start and end snapshots — included for reference
            so you can spot omissions or contributions from another
            actor.
          </>
        ) : (
          <>
            Files shown are the agent's declared authorship via{" "}
            <code>complete_task</code>/<code>amend_effort</code>{" "}
            so far. The effort hasn't ended yet, so the start↔end
            auto-diff isn't computable for these rows.
          </>
        )}
      </div>
    </section>
  );
}

const listStyle: React.CSSProperties = {
  margin: 0,
  paddingLeft: 18,
  fontSize: 11,
  color: "var(--text-secondary)",
};
const referenceListStyle: React.CSSProperties = {
  ...listStyle,
  color: "var(--text-muted)",
};

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

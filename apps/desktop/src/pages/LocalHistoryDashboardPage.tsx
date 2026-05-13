import { useCallback, useEffect, useMemo, useState } from "react";
import type { CommitRefLabel, EndingEffort, Snapshot, Stream } from "../api.js";
import {
  getSnapshotStats,
  getTaskSummaries,
  listEffortsEndingAtSnapshots,
  listSnapshots,
  resolveCommitRefLabels,
  subscribeSnapshotEvents,
} from "../api.js";
import { Card, cardLinkButton } from "../components/Card.js";
import { FileStatusCounts } from "../components/FileStatusCounts.js";
import { RefBadge } from "../components/RefBadge.js";
import { formatShortDateTime } from "../components/format.js";
import { logUi } from "../logger.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import type { NavSiblingEntry, NavSiblings } from "../tabs/PageNavigationContext.js";
import { gitCommitRef, snapshotRef, taskRef } from "../tabs/pageRefs.js";

const RECENT_LIMIT = 20;

export interface LocalHistoryDashboardPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean; siblings?: NavSiblings }): void;
}

interface SnapshotRowEffort {
  effortId: string;
  tasksId: number;
  title: string;
}

interface SnapshotRow {
  snapshot: Snapshot;
  summary: { created: number; modified: number; deleted: number; total: number } | null;
  efforts: SnapshotRowEffort[];
  /** True when this is the very first snapshot recorded for the
   *  stream — rendered as "Initial Snapshot" rather than the
   *  catch-all "External change" label. We can only assert this
   *  when the window we fetched is smaller than RECENT_LIMIT (i.e.
   *  no older snapshots scrolled off). */
  isInitial: boolean;
}

/** Pure label resolver for the snapshot row subject text. Extracted
 *  so the if/else logic is testable without a Card render. */
export function formatSnapshotSubject(
  efforts: ReadonlyArray<{ title: string }>,
  isInitial: boolean,
): string {
  if (efforts.length > 0) return efforts.map((e) => e.title).join(" · ");
  if (isInitial) return "Initial Snapshot";
  return "External change";
}

interface DashboardData {
  rows: SnapshotRow[];
  /** All branch+tag labels per snapshot's git commit sha. Absent shas
   *  fall back to a short-sha chip. */
  refLabels: Record<string, CommitRefLabel[]>;
}

/**
 * Local History dashboard — analogue of GitDashboardPage but driven
 * by snapshot rows (one per `request_snapshot()` call) instead of
 * git commits. Replaces the legacy per-file SnapshotsPanel.
 *
 * Layout mirrors GitDashboardPage: scrollable column of Cards. Each
 * card surfaces a different cut of the snapshot history; click into a
 * row to land on `SnapshotDetailPage` for the full file list and
 * per-file diff/restore.
 */
export function LocalHistoryDashboardPage({
  stream,
  onOpenPage,
}: LocalHistoryDashboardPageProps) {
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const streamId = stream?.id ?? null;

  const refresh = useCallback(async () => {
    if (!streamId) {
      setData(null);
      setLoading(false);
      return;
    }
    try {
      setError(null);
      const snapshots = await listSnapshots(streamId, RECENT_LIMIT);
      const snapshotIds = snapshots.map((s) => s.id);
      const [summaries, endingEfforts] = await Promise.all([
        Promise.all(
          snapshotIds.map(async (id) => {
            try {
              return [id, await getSnapshotStats(id)] as const;
            } catch (err) {
              logUi("warn", "snapshot stats fetch failed", { error: String(err), id });
              return [id, null] as const;
            }
          }),
        ),
        listEffortsEndingAtSnapshots(snapshotIds).catch((err): EndingEffort[] => {
          logUi("warn", "ending efforts fetch failed", { error: String(err) });
          return [];
        }),
      ]);
      const summaryById = new Map<number, { created: number; modified: number; deleted: number; total: number }>();
      for (const [id, s] of summaries) {
        if (s) summaryById.set(id, s);
      }
      // Resolve task titles for every effort the dashboard will show
      // — the efforts IPC only carries effort columns, no task title.
      const uniqueTaskIds = Array.from(new Set(endingEfforts.map((e) => e.tasksId)));
      const taskSummaries = await getTaskSummaries(uniqueTaskIds).catch((err) => {
        logUi("warn", "task summaries fetch failed", { error: String(err) });
        return [] as Array<{ id: number; title: string }>;
      });
      const titleByTaskId = new Map<number, string>(
        taskSummaries.map((t) => [t.id, t.title] as [number, string]),
      );
      const effortsBySnapshot = new Map<number, SnapshotRowEffort[]>();
      for (const e of endingEfforts) {
        const list = effortsBySnapshot.get(e.endSnapshotId) ?? [];
        list.push({
          effortId: e.effortId,
          tasksId: e.tasksId,
          title: titleByTaskId.get(e.tasksId) ?? `task ${e.tasksId}`,
        });
        effortsBySnapshot.set(e.endSnapshotId, list);
      }
      // The earliest snapshot in our window is the stream's first
      // snapshot only when we've fetched the entire history (no
      // older rows scrolled past RECENT_LIMIT). Without that guard
      // we'd falsely label the oldest visible row as "Initial".
      const sawFullHistory = snapshots.length < RECENT_LIMIT;
      const earliestId = sawFullHistory && snapshots.length > 0
        ? snapshots.reduce((min, s) => (s.id < min ? s.id : min), snapshots[0].id)
        : null;
      const rows: SnapshotRow[] = snapshots.map((snapshot) => ({
        snapshot,
        summary: summaryById.get(snapshot.id) ?? null,
        efforts: effortsBySnapshot.get(snapshot.id) ?? [],
        isInitial: earliestId !== null && snapshot.id === earliestId,
      }));
      const commitShas = Array.from(
        new Set(snapshots.map((s) => s.gitCommit).filter((sha): sha is string => Boolean(sha))),
      );
      const refLabels = await resolveCommitRefLabels(commitShas).catch(() => ({}));
      setData({ rows, refLabels });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [streamId]);

  useEffect(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);

  // Snapshot events fire on every `request_snapshot()` flush; the
  // batched event the writer emits is the one the dashboard cares
  // about. `subscribeSnapshotEvents` already coalesces both per-file
  // and batched variants into a single callback shape, so one
  // refresh covers either case.
  useEffect(() => {
    if (!streamId) return;
    const unsub = subscribeSnapshotEvents(streamId, () => {
      void refresh();
    });
    return () => unsub();
  }, [streamId, refresh]);

  const byBranch = useMemo(() => groupByBranch(data?.rows ?? []), [data?.rows]);

  if (!streamId) {
    return (
      <Page testId="page-local-history" title="Local History">
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-local-history" title="Local History">
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {error ? <div style={errorBanner}>{error}</div> : null}
        {loading && !data ? <div style={muted}>Loading…</div> : null}
        {data ? (
          <>
            <RecentSnapshotsCard
              rows={data.rows}
              onSelect={(id, siblings) => onOpenPage(snapshotRef(id), { siblings })}
              refLabels={data.refLabels}
            />
            {byBranch.length > 0 ? (
              <ByBranchCard groups={byBranch}
              onSelect={(id, siblings) => onOpenPage(snapshotRef(id), { siblings })}
              refLabels={data.refLabels} onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))} />
            ) : null}
            <RecentEffortsCard
              rows={data.rows}
              onOpenSnapshot={(id, siblings) => onOpenPage(snapshotRef(id), { siblings })}
              onOpenTask={(itemId) => onOpenPage(taskRef(itemId))}
            />
          </>
        ) : null}
      </div>
    </Page>
  );
}

function snapshotSiblingEntries(rows: SnapshotRow[]): NavSiblingEntry[] {
  return rows.map((row) => ({
    ref: snapshotRef(row.snapshot.id),
    label: formatSnapshotSubject(row.efforts, row.isInitial),
  }));
}

function RecentSnapshotsCard({
  rows,
  onSelect,
  refLabels,
}: {
  rows: SnapshotRow[];
  onSelect(id: number, siblings: NavSiblings): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  const entries = useMemo(() => snapshotSiblingEntries(rows), [rows]);
  return (
    <Card testId="local-history-recent" title="Recent Snapshots">
      {rows.length === 0 ? (
        <div style={muted}>No snapshots yet.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {rows.map((row, idx) => (
            <SnapshotRowItem
              key={row.snapshot.id}
              row={row}
              onSelect={(id) => onSelect(id, { entries, index: idx, title: "Recent snapshots" })}
              labels={row.snapshot.gitCommit ? refLabels[row.snapshot.gitCommit] ?? [] : []}
            />
          ))}
        </div>
      )}
    </Card>
  );
}

function SnapshotRowItem({
  row,
  onSelect,
  labels,
}: {
  row: SnapshotRow;
  onSelect(id: number): void;
  labels: CommitRefLabel[];
}) {
  const { snapshot, summary, efforts, isInitial } = row;
  const subjectish = formatSnapshotSubject(efforts, isInitial);
  return (
    <button
      type="button"
      data-testid="local-history-snapshot-row"
      onClick={() => onSelect(snapshot.id)}
      style={rowButtonStyle}
      title="Open snapshot detail"
    >
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {subjectish}
      </span>
      {labels.length > 0
        ? labels.map((l) => (
            <RefBadge key={`${l.kind}-${l.name}`} label={l.name} tone={l.kind} />
          ))
        : snapshot.gitCommit
        ? <RefBadge label={snapshot.gitCommit.slice(0, 7)} tone="sha" />
        : null}
      {summary ? (
        <FileStatusCounts
          filesAdded={summary.created}
          filesModified={summary.modified}
          filesDeleted={summary.deleted}
          title={`${summary.total} file${summary.total === 1 ? "" : "s"} captured: ${summary.created} created · ${summary.modified} modified · ${summary.deleted} deleted`}
        />
      ) : null}
      <span style={{ ...subtle, width: 130, flexShrink: 0, textAlign: "right" }} title={snapshot.createdAt}>
        {formatShortDateTime(snapshot.createdAt)}
      </span>
    </button>
  );
}

function ByBranchCard({
  groups,
  onSelect,
  onOpenCommit,
  refLabels,
}: {
  groups: Array<{ commit: string; rows: SnapshotRow[] }>;
  onSelect(id: number, siblings: NavSiblings): void;
  onOpenCommit(sha: string): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  return (
    <Card testId="local-history-by-branch" title="By Git Commit">
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {groups.map((group) => (
          <ByBranchGroup
            key={group.commit}
            group={group}
            onSelect={onSelect}
            onOpenCommit={onOpenCommit}
            refLabels={refLabels}
          />
        ))}
      </div>
    </Card>
  );
}

function ByBranchGroup({
  group,
  onSelect,
  onOpenCommit,
  refLabels,
}: {
  group: { commit: string; rows: SnapshotRow[] };
  onSelect(id: number, siblings: NavSiblings): void;
  onOpenCommit(sha: string): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  const entries = useMemo(() => snapshotSiblingEntries(group.rows), [group.rows]);
  const title = `Snapshots at ${group.commit.slice(0, 7)}`;
  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
        <button
          type="button"
          onClick={() => onOpenCommit(group.commit)}
          style={{ ...cardLinkButton, fontFamily: "monospace" }}
        >
          {group.commit.slice(0, 7)}
        </button>
        <span style={subtle}>· {group.rows.length} snapshots</span>
      </div>
      <div style={{ display: "flex", flexDirection: "column" }}>
        {group.rows.map((row, idx) => (
          <SnapshotRowItem
            key={row.snapshot.id}
            row={row}
            onSelect={(id) => onSelect(id, { entries, index: idx, title })}
            labels={row.snapshot.gitCommit ? refLabels[row.snapshot.gitCommit] ?? [] : []}
          />
        ))}
      </div>
    </div>
  );
}

function RecentEffortsCard({
  rows,
  onOpenSnapshot,
  onOpenTask,
}: {
  rows: SnapshotRow[];
  onOpenSnapshot(id: number, siblings: NavSiblings): void;
  onOpenTask(itemId: number): void;
}) {
  const flat = rows.flatMap((row) =>
    row.efforts.map((e) => ({ snapshot: row.snapshot, effort: e, isInitial: row.isInitial })),
  );
  const entries = useMemo<NavSiblingEntry[]>(
    () => flat.map((f) => ({
      ref: snapshotRef(f.snapshot.id),
      label: f.effort.title,
    })),
    [flat],
  );
  return (
    <Card testId="local-history-efforts" title="Recent Task Efforts">
      {flat.length === 0 ? (
        <div style={muted}>No task efforts landed in the recent snapshot window.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {flat.map(({ snapshot, effort }, idx) => (
            <div
              key={effort.effortId}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "4px 0",
                borderBottom: "1px solid var(--border-subtle)",
              }}
            >
              <span style={{ ...subtle, width: 130, flexShrink: 0 }}>
                {formatShortDateTime(snapshot.createdAt)}
              </span>
              <button
                type="button"
                onClick={() => onOpenTask(effort.tasksId)}
                style={{ ...cardLinkButton, flex: 1, minWidth: 0, textAlign: "left", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
                title={`task ${effort.tasksId}`}
              >
                {effort.title}
              </button>
              <button
                type="button"
                onClick={() => onOpenSnapshot(snapshot.id, { entries, index: idx, title: "Recent task efforts" })}
                style={cardLinkButton}
                title="Open snapshot detail"
              >
                snapshot {snapshot.id} →
              </button>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

function groupByBranch(
  rows: SnapshotRow[],
): Array<{ commit: string; rows: SnapshotRow[] }> {
  const byCommit = new Map<string, SnapshotRow[]>();
  for (const row of rows) {
    const commit = row.snapshot.gitCommit;
    if (!commit) continue;
    const existing = byCommit.get(commit) ?? [];
    existing.push(row);
    byCommit.set(commit, existing);
  }
  return Array.from(byCommit.entries())
    .filter(([, rs]) => rs.length >= 2)
    .map(([commit, rs]) => ({ commit, rows: rs }));
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)" };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-xs)" };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
const rowButtonStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "4px 6px",
  background: "transparent",
  border: "none",
  borderBottom: "1px solid var(--border-subtle)",
  cursor: "pointer",
  textAlign: "left",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
};

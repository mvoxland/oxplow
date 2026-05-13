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
import { gitCommitRef, snapshotRef, taskRef } from "../tabs/pageRefs.js";

const RECENT_LIMIT = 20;

export interface LocalHistoryDashboardPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
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
}

interface DashboardData {
  rows: SnapshotRow[];
  /** All branch+tag labels per pinned commit sha. Absent shas fall
   *  back to a short-sha chip. */
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
      const rows: SnapshotRow[] = snapshots.map((snapshot) => ({
        snapshot,
        summary: summaryById.get(snapshot.id) ?? null,
        efforts: effortsBySnapshot.get(snapshot.id) ?? [],
      }));
      const pinnedShas = Array.from(
        new Set(snapshots.map((s) => s.gitCommit).filter((sha): sha is string => Boolean(sha))),
      );
      const refLabels = await resolveCommitRefLabels(pinnedShas).catch(() => ({}));
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
              onSelect={(id) => onOpenPage(snapshotRef(id))}
              refLabels={data.refLabels}
            />
            {byBranch.length > 0 ? (
              <ByBranchCard groups={byBranch} onSelect={(id) => onOpenPage(snapshotRef(id))}
              refLabels={data.refLabels} onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))} />
            ) : null}
            <RecentEffortsCard
              rows={data.rows}
              onOpenSnapshot={(id) => onOpenPage(snapshotRef(id))}
              onOpenTask={(itemId) => onOpenPage(taskRef(itemId))}
            />
          </>
        ) : null}
      </div>
    </Page>
  );
}

function RecentSnapshotsCard({
  rows,
  onSelect,
  refLabels,
}: {
  rows: SnapshotRow[];
  onSelect(id: number): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  return (
    <Card testId="local-history-recent" title="Recent Snapshots">
      {rows.length === 0 ? (
        <div style={muted}>No snapshots yet.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {rows.map((row) => (
            <SnapshotRowItem
              key={row.snapshot.id}
              row={row}
              onSelect={onSelect}
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
  const { snapshot, summary, efforts } = row;
  const subjectish = efforts.length > 0
    ? efforts.map((e) => e.title).join(" · ")
    : "External change";
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
  onSelect(id: number): void;
  onOpenCommit(sha: string): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  return (
    <Card testId="local-history-by-branch" title="By Pinned Commit">
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {groups.map((group) => (
          <div key={group.commit}>
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
              {group.rows.map((row) => (
                <SnapshotRowItem
                  key={row.snapshot.id}
                  row={row}
                  onSelect={onSelect}
                  labels={row.snapshot.gitCommit ? refLabels[row.snapshot.gitCommit] ?? [] : []}
                />
              ))}
            </div>
          </div>
        ))}
      </div>
    </Card>
  );
}

function RecentEffortsCard({
  rows,
  onOpenSnapshot,
  onOpenTask,
}: {
  rows: SnapshotRow[];
  onOpenSnapshot(id: number): void;
  onOpenTask(itemId: number): void;
}) {
  const flat = rows.flatMap((row) =>
    row.efforts.map((e) => ({ snapshot: row.snapshot, effort: e })),
  );
  return (
    <Card testId="local-history-efforts" title="Recent Task Efforts">
      {flat.length === 0 ? (
        <div style={muted}>No task efforts landed in the recent snapshot window.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {flat.map(({ snapshot, effort }) => (
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
                onClick={() => onOpenSnapshot(snapshot.id)}
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

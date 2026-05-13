import { useCallback, useEffect, useMemo, useState } from "react";
import type { ParentSnapshot, Stream, TaskPriority, TaskStatus } from "../api.js";
import {
  getBlobStorageBytes,
  getParentSnapshotSummary,
  listEffortsEndingAtSnapshots,
  listParentSnapshots,
  subscribeSnapshotEvents,
} from "../api.js";
import { Card, cardLinkButton } from "../components/Card.js";
import { formatBytes, formatShortDateTime } from "../components/format.js";
import { logUi } from "../logger.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, snapshotRef, taskRef } from "../tabs/pageRefs.js";

const RECENT_LIMIT = 20;

export interface LocalHistoryDashboardPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
}

interface SnapshotRow {
  parent: ParentSnapshot;
  summary: { created: number; modified: number; deleted: number; total: number } | null;
  efforts: Array<{
    effortId: string;
    tasksId: number;
    threadId: string;
    title: string;
    status: TaskStatus;
    priority: TaskPriority;
  }>;
}

interface DashboardData {
  rows: SnapshotRow[];
  storage: {
    totalSnapshots: number;
    blobBytes: number;
  };
}

/**
 * Local History dashboard — analogue of GitDashboardPage but driven
 * by parent snapshot rows (one per `request_snapshot()` call) instead
 * of git commits. Replaces the legacy per-file SnapshotsPanel.
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
      const [parents, blobBytes] = await Promise.all([
        listParentSnapshots(streamId, RECENT_LIMIT),
        getBlobStorageBytes().catch(() => 0),
      ]);
      const parentIds = parents.map((p) => p.id);
      const [summaries, effortsByParent] = await Promise.all([
        Promise.all(
          parentIds.map(async (id) => {
            try {
              return [id, await getParentSnapshotSummary(id)] as const;
            } catch (err) {
              logUi("warn", "snapshot summary fetch failed", { error: String(err), id });
              return [id, null] as const;
            }
          }),
        ),
        listEffortsEndingAtSnapshots(parentIds.map(String)).catch(() => ({})),
      ]);
      const summaryById = new Map<number, { created: number; modified: number; deleted: number; total: number }>();
      for (const [id, s] of summaries) {
        if (s) summaryById.set(id, s);
      }
      const rows: SnapshotRow[] = parents.map((parent) => ({
        parent,
        summary: summaryById.get(parent.id) ?? null,
        efforts: effortsByParent[String(parent.id)] ?? [],
      }));
      setData({
        rows,
        storage: { totalSnapshots: parents.length, blobBytes },
      });
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
            <StorageCard storage={data.storage} />
            <RecentSnapshotsCard
              rows={data.rows}
              onSelect={(id) => onOpenPage(snapshotRef(id))}
            />
            {byBranch.length > 0 ? (
              <ByBranchCard groups={byBranch} onSelect={(id) => onOpenPage(snapshotRef(id))} onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))} />
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

function StorageCard({
  storage,
}: {
  storage: DashboardData["storage"];
}) {
  return (
    <Card testId="local-history-storage" title="Storage">
      <div style={{ display: "flex", gap: 18, fontSize: "var(--text-sm)" }}>
        <div>
          <div style={subtle}>Recent snapshots</div>
          <div style={{ fontWeight: "var(--weight-medium)" }}>{storage.totalSnapshots}</div>
        </div>
        <div>
          <div style={subtle}>Blob storage</div>
          <div style={{ fontWeight: "var(--weight-medium)" }}>{formatBytes(storage.blobBytes)}</div>
        </div>
      </div>
    </Card>
  );
}

function RecentSnapshotsCard({
  rows,
  onSelect,
}: {
  rows: SnapshotRow[];
  onSelect(id: number): void;
}) {
  return (
    <Card testId="local-history-recent" title="Recent Snapshots">
      {rows.length === 0 ? (
        <div style={muted}>No snapshots yet.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {rows.map((row) => (
            <SnapshotRowItem key={row.parent.id} row={row} onSelect={onSelect} />
          ))}
        </div>
      )}
    </Card>
  );
}

function SnapshotRowItem({
  row,
  onSelect,
}: {
  row: SnapshotRow;
  onSelect(id: number): void;
}) {
  const { parent, summary, efforts } = row;
  const subjectish = efforts.length > 0
    ? efforts.map((e) => e.title).join(" · ")
    : parent.gitCommit
    ? `Clean tree at ${parent.gitCommit.slice(0, 7)}`
    : "External change";
  return (
    <button
      type="button"
      data-testid="local-history-snapshot-row"
      onClick={() => onSelect(parent.id)}
      style={rowButtonStyle}
      title="Open snapshot detail"
    >
      <span style={{ ...subtle, width: 130, flexShrink: 0 }} title={parent.createdAt}>
        {formatShortDateTime(parent.createdAt)}
      </span>
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {subjectish}
      </span>
      {summary ? (
        <span style={{ ...subtle, fontFamily: "monospace", whiteSpace: "nowrap" }}
          title={`${summary.total} files captured`}>
          +{summary.created} ~{summary.modified} −{summary.deleted}
        </span>
      ) : (
        <span style={{ ...subtle, fontFamily: "monospace" }}>{parent.fileCount} files</span>
      )}
    </button>
  );
}

function ByBranchCard({
  groups,
  onSelect,
  onOpenCommit,
}: {
  groups: Array<{ commit: string; rows: SnapshotRow[] }>;
  onSelect(id: number): void;
  onOpenCommit(sha: string): void;
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
                <SnapshotRowItem key={row.parent.id} row={row} onSelect={onSelect} />
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
    row.efforts.map((e) => ({ parent: row.parent, effort: e })),
  );
  return (
    <Card testId="local-history-efforts" title="Recent Task Efforts">
      {flat.length === 0 ? (
        <div style={muted}>No task efforts landed in the recent snapshot window.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {flat.map(({ parent, effort }) => (
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
                {formatShortDateTime(parent.createdAt)}
              </span>
              <button
                type="button"
                onClick={() => onOpenTask(effort.tasksId)}
                style={{ ...cardLinkButton, flex: 1, minWidth: 0, textAlign: "left", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
                title={`task ${effort.tasksId} · ${effort.status}`}
              >
                {effort.title}
              </button>
              <button
                type="button"
                onClick={() => onOpenSnapshot(parent.id)}
                style={cardLinkButton}
                title="Open snapshot detail"
              >
                snapshot {parent.id} →
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
    const commit = row.parent.gitCommit;
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

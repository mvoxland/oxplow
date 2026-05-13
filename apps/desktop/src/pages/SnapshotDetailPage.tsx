import { useEffect, useMemo, useState } from "react";
import type { ParentSnapshot, ParentSnapshotFile, Stream } from "../api.js";
import {
  getParentSnapshotSummary,
  getSnapshotPairDiff,
  listFilesForSnapshot,
  listParentSnapshots,
  restoreFileFromSnapshot,
} from "../api.js";
import { Card } from "../components/Card.js";
import { DISK } from "../file-version.js";
import { logUi } from "../logger.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef } from "../tabs/pageRefs.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { formatBytes, formatFullDateTime } from "../components/format.js";

export interface SnapshotDetailPageProps {
  stream: Stream | null;
  snapshotId: number;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
}

interface DetailData {
  parent: ParentSnapshot;
  summary: { created: number; modified: number; deleted: number; total: number };
  files: ParentSnapshotFile[];
}

/**
 * Single parent-snapshot page — drilled into from the Local History
 * dashboard's Recent Snapshots card. Mirrors the GitCommitPage shape:
 * a SummaryRow card at the top, a metadata block (timestamp, git pin),
 * then the file list with click → per-file diff against the previous
 * capture and right-click → restore.
 *
 * Reuses the shared `Card` shell + `formatBytes`/`formatFullDateTime`
 * helpers so it slots into the same visual vocabulary as the Git
 * pages and the dashboards.
 */
export function SnapshotDetailPage({
  stream,
  snapshotId,
  onOpenDiff,
  onOpenPage,
}: SnapshotDetailPageProps) {
  const [data, setData] = useState<DetailData | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!stream) {
      setData(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    void Promise.all([
      // The IPC for "get one parent" doesn't exist; pull the recent
      // window and pick our id. Cheap (~500 rows) and avoids a new
      // backend command for what's effectively a list lookup.
      listParentSnapshots(stream.id, 500).then(
        (rows) => rows.find((r) => r.id === snapshotId) ?? null,
      ),
      getParentSnapshotSummary(snapshotId),
      listFilesForSnapshot(snapshotId),
    ])
      .then(([parent, summary, files]) => {
        if (cancelled) return;
        if (!parent) {
          setError(`Snapshot ${snapshotId} not found.`);
          setData(null);
        } else {
          setData({ parent, summary, files });
        }
        setLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot detail load failed", { error: String(err) });
        setError(String(err));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [stream?.id, snapshotId]);

  const handleOpenFileDiff = async (file: ParentSnapshotFile) => {
    if (!onOpenDiff) return;
    try {
      const result = await getSnapshotPairDiff(null, String(snapshotId), file.path);
      onOpenDiff({
        path: file.path,
        leftVersion: DISK,
        rightVersion: DISK,
        baseLabel: `prev → snapshot ${snapshotId}`,
        leftContent: result.before ?? "",
        rightContent: result.after ?? "",
        labelOverride: `${file.path} @ snapshot ${snapshotId}`,
      });
    } catch (err) {
      logUi("warn", "snapshot file diff failed", { error: String(err) });
    }
  };

  const handleRestore = async (file: ParentSnapshotFile) => {
    if (!stream) return;
    if (file.blobHash == null) return;
    const ok = window.confirm(
      `Restore ${file.path} from snapshot ${snapshotId}? This overwrites the working-tree file.`,
    );
    if (!ok) return;
    try {
      await restoreFileFromSnapshot(stream.id, String(file.id), file.path);
    } catch (err) {
      logUi("warn", "restore snapshot file failed", { error: String(err) });
      recordOpError({
        label: `Restore from snapshot: ${file.path}`,
        message: String(err),
      });
    }
  };

  const title = useMemo(() => {
    if (!data) return `Snapshot ${snapshotId}`;
    return `Snapshot ${snapshotId} · ${formatFullDateTime(data.parent.createdAt)}`;
  }, [data, snapshotId]);

  return (
    <Page testId="page-snapshot-detail" title={title}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {error ? <div style={errorBanner}>{error}</div> : null}
        {loading && !data ? <div style={muted}>Loading…</div> : null}
        {data ? (
          <>
            <SnapshotSummaryCard parent={data.parent} summary={data.summary} onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))} />
            <Card testId="snapshot-detail-files" title={`Files (${data.files.length})`}>
              {data.files.length === 0 ? (
                <div style={muted}>No files captured.</div>
              ) : (
                <div style={{ display: "flex", flexDirection: "column" }}>
                  {data.files.map((file) => {
                    const status = file.blobHash == null
                      ? "deleted"
                      : file.oversize
                      ? "oversize"
                      : "content";
                    return (
                      <button
                        key={file.id}
                        type="button"
                        data-testid="snapshot-detail-file-row"
                        onClick={() => handleOpenFileDiff(file)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          void handleRestore(file);
                        }}
                        style={rowStyle}
                        title="Click to diff against the prior capture · Right-click to restore"
                      >
                        <span style={{ ...statusBadgeStyle, ...statusColors[status] }}>
                          {statusGlyph[status]}
                        </span>
                        <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                          {file.path}
                        </span>
                        <span style={subtle}>{formatBytes(file.sizeBytes)}</span>
                      </button>
                    );
                  })}
                </div>
              )}
            </Card>
          </>
        ) : null}
      </div>
    </Page>
  );
}

/**
 * Snapshot-flavored summary row, structured the same way as
 * `ChangeAnalysis/SummaryCard` (uppercase mini-label above big value)
 * so the two pages read identically when sitting side-by-side. The
 * stat axes differ — snapshots don't carry per-line additions — so
 * this is its own component rather than reusing the git one verbatim.
 */
function SnapshotSummaryCard({
  parent,
  summary,
  onOpenCommit,
}: {
  parent: ParentSnapshot;
  summary: { created: number; modified: number; deleted: number; total: number };
  onOpenCommit(sha: string): void;
}) {
  return (
    <section data-testid="snapshot-summary" style={summaryCard}>
      <div style={summaryHeader}>Summary</div>
      <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
        <Stat label="Files" value={summary.total} />
        <Stat label="Created" value={summary.created} color="var(--text-success, #16a34a)" />
        <Stat label="Modified" value={summary.modified} />
        <Stat label="Deleted" value={summary.deleted} color="var(--text-danger, #dc2626)" />
      </div>
      <div style={{ marginTop: 12, fontSize: 12, color: "var(--text-muted)" }}>
        Captured {formatFullDateTime(parent.createdAt)}
        {parent.gitCommit ? (
          <>
            {" · pinned to commit "}
            <button
              type="button"
              onClick={() => onOpenCommit(parent.gitCommit!)}
              style={linkButton}
            >
              {parent.gitCommit.slice(0, 7)}
            </button>
          </>
        ) : (
          " · worktree was dirty (no commit pinned)"
        )}
      </div>
    </section>
  );
}

function Stat({ label, value, color }: { label: string; value: number; color?: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", minWidth: 64 }}>
      <span style={{ fontSize: 11, color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: 0.5 }}>
        {label}
      </span>
      <span style={{ fontSize: 18, fontWeight: 600, color: color ?? "var(--text-primary)" }}>{value}</span>
    </div>
  );
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13 };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
const linkButton: React.CSSProperties = {
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  fontFamily: "monospace",
  fontSize: 12,
  cursor: "pointer",
};
const rowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "4px 6px",
  background: "transparent",
  border: "none",
  borderBottom: "1px solid var(--border-subtle)",
  cursor: "pointer",
  textAlign: "left",
  fontSize: 13,
  color: "var(--text-primary)",
};
const statusBadgeStyle: React.CSSProperties = {
  fontFamily: "monospace",
  fontSize: 11,
  fontWeight: 600,
  width: 16,
  textAlign: "center",
};
const statusColors: Record<string, React.CSSProperties> = {
  content: { color: "var(--text-muted)" },
  deleted: { color: "var(--text-warning, #dc2626)" },
  oversize: { color: "var(--text-warning, #92400e)" },
};
const statusGlyph: Record<string, string> = {
  content: "·",
  deleted: "−",
  oversize: "≫",
};
const summaryCard: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const summaryHeader: React.CSSProperties = { fontWeight: 600, marginBottom: 8 };

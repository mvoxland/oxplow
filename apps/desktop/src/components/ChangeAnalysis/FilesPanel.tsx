import { useMemo, useState } from "react";
import type { BranchChangeEntry, GitFileStatus } from "../../api-types.js";
import type {
  FilePivots,
  FunctionChurnRow,
  FunctionsBuckets,
} from "./analysisHelpers.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import {
  changeAnalysisRef,
  type ChangeAnalysisScope,
  type ChangeAnalysisTarget,
} from "../../tabs/pageRefs.js";
import type { NavSiblingEntry } from "../../tabs/PageNavigationContext.js";
import { FunctionsCard } from "./FunctionsCard.js";
import { ChangeAnalysisFileTree } from "./FileTreeView.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";

/** Multi-select status filter. Empty == no rows; default = all three
 *  checked. Stored as a Set so toggling is independent. */
export type StatusKey = "added" | "modified" | "deleted";
export const ALL_STATUSES: readonly StatusKey[] = ["added", "modified", "deleted"];

export type FilesPanelView = "extension" | "files" | "semantic";

interface FilesPanelProps {
  /** Files / functions / pivots already filtered by `statusFilter`
   *  in the host. The panel itself only chooses how to render. */
  files: BranchChangeEntry[];
  functions: FunctionsBuckets;
  /** Per-function line churn — drives the +/− columns + bar in
   *  the Semantic view. */
  functionChurn: FunctionChurnRow[];
  pivots: FilePivots;
  /** Status filter state, owned by the host because it also drives
   *  the smell panels below. The panel surfaces the toolbar
   *  checkboxes and calls `onStatusChange` to flip a single key. */
  statusFilter: Set<StatusKey>;
  onStatusChange(next: Set<StatusKey>): void;
  /** The host target — used to build extension drilldown refs. */
  target: ChangeAnalysisTarget;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** Function-row "open diff" action, supplied by the drilldown. */
  onOpenFunctionDiff(path: string, line: number): void;
  onOpenFileDiff(path: string): void;
}

/**
 * Single combined "Files" panel that fronts the analysis content:
 *
 *   - View toggle: By extension | Files (tree) | Semantic
 *   - Status checkboxes: Added | Modified | Deleted (multi-select,
 *     all checked by default)
 *
 * Replaces the prior pair of panels (a top FilesPivot with three
 * sub-pivots, plus a separate Semantic / File-list card below).
 * Status filter affects every smell panel below this one too — the
 * host owns the state and slices everything by it.
 */
export function FilesPanel({
  files,
  functions,
  functionChurn,
  pivots,
  statusFilter,
  onStatusChange,
  target,
  onOpenFile,
  onOpenFunctionDiff,
  onOpenFileDiff,
}: FilesPanelProps) {
  const [view, setView] = useState<FilesPanelView>("files");
  // Persist the view selection across reloads.
  usePageSnapshot<{ filesView: FilesPanelView }>({
    serialize: () => ({ filesView: view }),
    restore: (snap) => {
      if (snap.filesView === "extension" || snap.filesView === "files" || snap.filesView === "semantic") {
        setView(snap.filesView);
      }
    },
    deps: [view],
  });

  const toggleStatus = (key: StatusKey) => {
    const next = new Set(statusFilter);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    onStatusChange(next);
  };

  return (
    <section data-testid="change-analysis-files" style={card}>
      <div style={toolbarRow}>
        <div style={{ display: "flex", gap: 4 }}>
          {([
            ["extension", "By extension"],
            ["files", "Files"],
            ["semantic", "Functions"],
          ] as const).map(([key, label]) => (
            <button
              key={key}
              type="button"
              data-testid={`change-analysis-view-${key}`}
              onClick={() => setView(key)}
              style={view === key ? activeTab : tab}
            >
              {label}
            </button>
          ))}
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          {ALL_STATUSES.map((key) => (
            <label
              key={key}
              data-testid={`change-analysis-status-${key}`}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 4,
                fontSize: 12,
                cursor: "pointer",
                color: statusFilter.has(key) ? "var(--text-primary)" : "var(--text-muted)",
              }}
            >
              <input
                type="checkbox"
                checked={statusFilter.has(key)}
                onChange={() => toggleStatus(key)}
              />
              {capitalize(key)}
            </label>
          ))}
        </div>
      </div>

      {view === "extension" ? (
        <ExtensionPivot pivots={pivots} target={target} />
      ) : view === "files" ? (
        files.length === 0 ? (
          <div style={muted}>No files match the current status filter.</div>
        ) : (
          <ChangeAnalysisFileTree
            files={files}
            target={target}
            onOpenFile={onOpenFile}
            onOpenFileDiff={onOpenFileDiff}
          />
        )
      ) : (
        files.length === 0 ? (
          <div style={muted}>No files match the current status filter.</div>
        ) : (
          <FunctionsCard
            functions={functions}
            churn={functionChurn}
            target={target}
            onOpenFile={onOpenFile}
            onOpenFunctionDiff={onOpenFunctionDiff}
          />
        )
      )}
    </section>
  );
}

function ExtensionPivot({
  pivots,
  target,
}: {
  pivots: FilePivots;
  target: ChangeAnalysisTarget;
}) {
  const rows = pivots.byExtension;
  const maxFiles = rows.reduce((m, r) => Math.max(m, r.files), 1);
  const siblingEntries: NavSiblingEntry[] = useMemo(
    () => rows.map((r) => ({
      ref: changeAnalysisRef(target, { kind: "ext", value: r.key } as ChangeAnalysisScope),
      label: r.key,
    })),
    [rows, target],
  );

  if (rows.length === 0) {
    return <div style={muted}>Nothing to pivot.</div>;
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      {rows.map((row, idx) => (
        <PivotRow
          key={row.key}
          rowKey={row.key}
          files={row.files}
          additions={row.additions}
          deletions={row.deletions}
          byStatus={row.byStatus}
          maxFiles={maxFiles}
          target={target}
          scope={{ kind: "ext", value: row.key }}
          siblings={{ entries: siblingEntries, index: idx, title: "Files by extension" }}
        />
      ))}
    </div>
  );
}

function PivotRow({
  rowKey,
  files,
  additions,
  deletions,
  byStatus,
  maxFiles,
  target,
  scope,
  siblings,
}: {
  rowKey: string;
  files: number;
  additions: number;
  deletions: number;
  byStatus: { added: number; modified: number; deleted: number };
  maxFiles: number;
  target: ChangeAnalysisTarget;
  scope: ChangeAnalysisScope;
  siblings: { entries: NavSiblingEntry[]; index: number; title?: string };
}) {
  const ref = changeAnalysisRef(target, scope);
  const { handlers } = useRouteDispatch(ref, { siblings });
  // Total bar length scales the row's file count against the max
  // file count in the pivot. Within that length, three colored
  // segments (added / modified / deleted) split proportionally to
  // each status's share of the row.
  // Track is fixed-width; colored fill scales to (files / maxFiles)
  // so a row with fewer files shows a shorter colored portion. Per-
  // status segments split the colored fill proportionally.
  const totalPct = maxFiles === 0 ? 0 : (files / maxFiles) * 100;
  const addPct = files === 0 ? 0 : (byStatus.added / files) * 100;
  const modPct = files === 0 ? 0 : (byStatus.modified / files) * 100;
  const delPct = files === 0 ? 0 : (byStatus.deleted / files) * 100;
  const tooltip = `${rowKey}: ${byStatus.added} added · ${byStatus.modified} modified · ${byStatus.deleted} deleted`;
  return (
    <button
      type="button"
      data-testid="change-analysis-pivot-row"
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      title={tooltip}
      style={pivotRowButton}
    >
      <span style={addCol}>{additions > 0 ? `+${additions}` : ""}</span>
      <span style={delCol}>{deletions > 0 ? `−${deletions}` : ""}</span>
      <div style={barTrack}>
        <div style={{ display: "flex", height: "100%", width: `${totalPct}%` }}>
          {byStatus.added > 0 ? (
            <span style={{ width: `${addPct}%`, background: "var(--text-success, #16a34a)" }} />
          ) : null}
          {byStatus.modified > 0 ? (
            <span style={{ width: `${modPct}%`, background: "var(--text-warning, #d97706)" }} />
          ) : null}
          {byStatus.deleted > 0 ? (
            <span style={{ width: `${delPct}%`, background: "var(--text-danger, #dc2626)" }} />
          ) : null}
        </div>
      </div>
      <span style={{ fontWeight: 500 }}>{rowKey}</span>
      <span style={{ color: "var(--text-muted)" }}>
        {files} file{files === 1 ? "" : "s"}
      </span>
    </button>
  );
}

/** Pure helper: does `status` pass the current Set filter? Empty
 *  Set = nothing passes; full Set = everything passes. */
export function statusPasses(status: GitFileStatus, filter: Set<StatusKey>): boolean {
  if (filter.size === 0) return false;
  if (status === "added" || status === "untracked") return filter.has("added");
  if (status === "modified" || status === "renamed") return filter.has("modified");
  if (status === "deleted") return filter.has("deleted");
  return true;
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const toolbarRow: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
  marginBottom: 12,
  flexWrap: "wrap",
  gap: 8,
};
const tab: React.CSSProperties = {
  padding: "4px 10px",
  background: "transparent",
  color: "var(--text-muted)",
  borderWidth: 1,
  borderStyle: "solid",
  borderColor: "var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 12,
};
const activeTab: React.CSSProperties = {
  ...tab,
  background: "var(--accent-soft-bg, var(--surface-app))",
  color: "var(--text-primary)",
  borderColor: "var(--text-link, #2563eb)",
};
const barTrack: React.CSSProperties = {
  width: 100,
  height: 8,
  background: "var(--surface-app)",
  borderRadius: 4,
  overflow: "hidden",
  flexShrink: 0,
};
const addCol: React.CSSProperties = {
  color: "var(--text-success, #16a34a)",
  width: 44,
  textAlign: "right",
  fontSize: 11,
  fontFamily: "ui-monospace, monospace",
  flexShrink: 0,
};
const delCol: React.CSSProperties = {
  color: "var(--text-danger, #dc2626)",
  width: 44,
  textAlign: "right",
  fontSize: 11,
  fontFamily: "ui-monospace, monospace",
  flexShrink: 0,
};
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13 };
const pivotRowButton: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  fontSize: 13,
  textAlign: "left",
  background: "transparent",
  border: "none",
  borderRadius: 4,
  padding: "4px 6px",
  cursor: "pointer",
  color: "var(--text-primary)",
  width: "100%",
};

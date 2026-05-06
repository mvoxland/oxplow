import { useMemo, useState } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import type { FunctionChurnRow } from "./analysisHelpers.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";

type View = "files" | "functions";
type Sort = "total" | "added" | "deleted";

interface ChurnCardProps {
  files: BranchChangeEntry[];
  functionChurn: FunctionChurnRow[];
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
}

interface UnifiedRow {
  key: string;
  path: string;
  /** Function qualified name when the row represents a function. */
  fnLabel: string | null;
  added: number;
  deleted: number;
  /** Share of the active view's total churn (0..1). */
  share: number;
}

const FILE_ROW_CAP = 15;
const FUNCTION_ROW_CAP = 20;
const MIN_FUNCTION_LINES = 5;
const MIN_FUNCTION_SHARE = 0.05;

/**
 * Combined "churn" panel — one card with a Files / Functions
 * toggle. Both views share the same row shape so the eye scans
 * cleanly between them: stacked status bar (added vs deleted
 * proportions, scaled to the row's share of the active view's
 * total churn), +/− line columns, share-of-churn percent, then
 * the filename + optional function name on the right.
 */
export function ChurnCard({ files, functionChurn, onOpenFile }: ChurnCardProps) {
  const [view, setView] = useState<View>("files");
  const [sort, setSort] = useState<Sort>("total");
  usePageSnapshot<{ churnView: View; churnSort: Sort }>({
    serialize: () => ({ churnView: view, churnSort: sort }),
    restore: (snap) => {
      if (snap.churnView === "files" || snap.churnView === "functions") setView(snap.churnView);
      if (snap.churnSort === "total" || snap.churnSort === "added" || snap.churnSort === "deleted") {
        setSort(snap.churnSort);
      }
    },
    deps: [view, sort],
  });
  const compare = (a: UnifiedRow, b: UnifiedRow): number => {
    if (sort === "added") return b.added - a.added;
    if (sort === "deleted") return b.deleted - a.deleted;
    return b.added + b.deleted - (a.added + a.deleted);
  };

  const rows = useMemo<UnifiedRow[]>(() => {
    if (view === "files") {
      const totalChurn = files.reduce(
        (acc, f) => acc + (f.additions ?? 0) + (f.deletions ?? 0),
        0,
      );
      return [...files]
        .map((f) => {
          const added = f.additions ?? 0;
          const deleted = f.deletions ?? 0;
          const total = added + deleted;
          return {
            key: `file::${f.path}`,
            path: f.path,
            fnLabel: null,
            added,
            deleted,
            share: totalChurn === 0 ? 0 : total / totalChurn,
          };
        })
        .filter((r) => r.added + r.deleted > 0)
        .sort(compare)
        .slice(0, FILE_ROW_CAP);
    }
    // functions
    const totalChurn = functionChurn.reduce(
      (acc, c) => acc + c.addedLines + c.deletedLines,
      0,
    );
    return functionChurn
      .map<UnifiedRow>((c) => {
        const total = c.addedLines + c.deletedLines;
        return {
          key: `fn::${c.path}::${c.containerPath.join("::")}::${c.name}`,
          path: c.path,
          fnLabel: c.containerPath.length > 0
            ? `${c.containerPath.join("::")}::${c.name}`
            : c.name,
          added: c.addedLines,
          deleted: c.deletedLines,
          share: totalChurn === 0 ? 0 : total / totalChurn,
        };
      })
      .filter((r) => r.added + r.deleted >= MIN_FUNCTION_LINES || r.share >= MIN_FUNCTION_SHARE)
      .sort(compare)
      .slice(0, FUNCTION_ROW_CAP);
  }, [view, files, functionChurn, sort]);

  if (rows.length === 0 && view === "files" && files.length === 0) return null;

  return (
    <section data-testid="change-analysis-churn" style={card}>
      <div style={toolbarRow}>
        <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
          <div style={header}>Churn</div>
          <div
            style={{ display: "flex", gap: 4, alignItems: "center" }}
            title="Sort the rows by total line churn, additions only, or deletions only. The % column always shows share of total churn."
          >
            <span style={{ fontSize: 11, color: "var(--text-muted)" }}>Sort:</span>
            {([
              ["total", "Total"],
              ["added", "+ Added"],
              ["deleted", "− Deleted"],
            ] as const).map(([key, label]) => (
              <button
                key={key}
                type="button"
                data-testid={`change-analysis-churn-sort-${key}`}
                onClick={() => setSort(key)}
                style={sort === key ? activeTab : tab}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
        <div style={{ display: "flex", gap: 4 }}>
          {([
            ["files", "Files"],
            ["functions", "Functions"],
          ] as const).map(([key, label]) => (
            <button
              key={key}
              type="button"
              data-testid={`change-analysis-churn-view-${key}`}
              onClick={() => setView(key)}
              style={view === key ? activeTab : tab}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      {rows.length === 0 ? (
        <div style={muted}>
          {view === "functions"
            ? "No function accounts for ≥5% of diff churn — changes are spread evenly."
            : "Nothing to show."}
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {rows.map((row) => (
            <ChurnRow key={row.key} row={row} onOpenFile={onOpenFile} />
          ))}
        </div>
      )}
    </section>
  );
}

function ChurnRow({
  row,
  onOpenFile,
}: {
  row: UnifiedRow;
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
}) {
  const total = row.added + row.deleted;
  // Bar fills its track proportional to the row's share of total
  // churn, then splits internally into a green (added) / red
  // (deleted) segment by line count. Shares < ~3% still get a
  // visible sliver thanks to the floor.
  const fillPct = Math.max(row.share * 100, total > 0 ? 2 : 0);
  const addPct = total === 0 ? 0 : (row.added / total) * 100;
  const delPct = 100 - addPct;
  const tooltip = `+${row.added} added · −${row.deleted} deleted · ${(row.share * 100).toFixed(1)}% of ${row.fnLabel ? "function" : "file"} churn`;
  return (
    <div
      data-testid="change-analysis-churn-row"
      style={rowOuter}
      title={tooltip}
    >
      <span style={addCol}>{row.added > 0 ? `+${row.added}` : ""}</span>
      <span style={delCol}>{row.deleted > 0 ? `−${row.deleted}` : ""}</span>
      <div style={barTrack}>
        <div style={{ display: "flex", height: "100%", width: `${fillPct}%` }}>
          {row.added > 0 ? (
            <span style={{ width: `${addPct}%`, background: "var(--text-success, #16a34a)" }} />
          ) : null}
          {row.deleted > 0 ? (
            <span style={{ width: `${delPct}%`, background: "var(--text-danger, #dc2626)" }} />
          ) : null}
        </div>
      </div>
      <span style={pctCol}>{(row.share * 100).toFixed(0)}%</span>
      <button
        type="button"
        onClick={(e) => onOpenFile?.(row.path, { newTab: e.metaKey || e.ctrlKey })}
        style={pathButton}
        title={row.path}
      >
        {row.path}
      </button>
      {row.fnLabel ? (
        <span style={fnCol} title={row.fnLabel}>
          {row.fnLabel}
        </span>
      ) : null}
    </div>
  );
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600 };
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
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13 };
const rowOuter: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  fontSize: 12,
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
const barTrack: React.CSSProperties = {
  width: 120,
  height: 8,
  background: "var(--surface-app)",
  borderRadius: 4,
  overflow: "hidden",
  flexShrink: 0,
};
const pctCol: React.CSSProperties = {
  width: 36,
  textAlign: "right",
  color: "var(--text-muted)",
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
  flexShrink: 0,
};
const pathButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: 12,
  textAlign: "left",
  flexShrink: 0,
  maxWidth: 320,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
const fnCol: React.CSSProperties = {
  color: "var(--text-primary)",
  fontWeight: 500,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
  minWidth: 0,
};

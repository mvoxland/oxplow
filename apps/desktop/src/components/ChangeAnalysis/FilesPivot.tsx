import { useMemo, useState } from "react";
import type { FilePivots } from "./analysisHelpers.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import { changeAnalysisRef, type ChangeAnalysisScope, type ChangeAnalysisTarget } from "../../tabs/pageRefs.js";
import type { NavSiblingEntry } from "../../tabs/PageNavigationContext.js";

type PivotKey = "extension" | "directory" | "status";

interface FilesPivotProps {
  pivots: FilePivots;
  /** The analysis target the rows should drill into. */
  target: ChangeAnalysisTarget;
}

export function FilesPivot({ pivots, target }: FilesPivotProps) {
  const [active, setActive] = useState<PivotKey>("extension");
  const rows = useMemo(() => {
    if (active === "extension") return pivots.byExtension;
    if (active === "directory") return pivots.byTopDir;
    // status pivot: synthesize rows from byStatus map.
    return Object.entries(pivots.byStatus)
      .filter(([, n]) => n > 0)
      .map(([k, n]) => ({ key: k, files: n, additions: 0, deletions: 0 }));
  }, [active, pivots]);
  const maxFiles = rows.reduce((m, r) => Math.max(m, r.files), 1);

  // Each row in the active pivot routes to the focused drilldown for
  // that scope. Siblings = the rest of the visible rows in this pivot,
  // so the drilldown gets up/down navigation between e.g. all
  // extensions or all directories without going back to the dashboard.
  const scopeKindForPivot: Record<PivotKey, ChangeAnalysisScope["kind"]> = {
    extension: "ext",
    directory: "dir",
    status: "status",
  };
  const scopeKind = scopeKindForPivot[active];
  const siblingEntries: NavSiblingEntry[] = useMemo(
    () => rows.map((r) => ({
      ref: changeAnalysisRef(target, { kind: scopeKind, value: r.key } as ChangeAnalysisScope),
      label: r.key,
    })),
    [rows, scopeKind, target],
  );

  return (
    <section data-testid="change-analysis-files" style={card}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
        <div style={header}>Files</div>
        <div style={{ display: "flex", gap: 4 }}>
          {(["extension", "directory", "status"] as PivotKey[]).map((key) => (
            <button
              key={key}
              type="button"
              data-testid={`change-analysis-pivot-${key}`}
              onClick={() => setActive(key)}
              style={active === key ? activeTab : tab}
            >
              By {key}
            </button>
          ))}
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {rows.length === 0 ? (
          <div style={muted}>Nothing to pivot.</div>
        ) : (
          rows.map((row, idx) => (
            <PivotRow
              key={row.key}
              rowKey={row.key}
              files={row.files}
              additions={row.additions}
              deletions={row.deletions}
              maxFiles={maxFiles}
              showAddDel={active !== "status"}
              target={target}
              scope={{ kind: scopeKind, value: row.key } as ChangeAnalysisScope}
              siblings={{ entries: siblingEntries, index: idx }}
            />
          ))
        )}
      </div>
    </section>
  );
}

function PivotRow({
  rowKey,
  files,
  additions,
  deletions,
  maxFiles,
  showAddDel,
  target,
  scope,
  siblings,
}: {
  rowKey: string;
  files: number;
  additions: number;
  deletions: number;
  maxFiles: number;
  showAddDel: boolean;
  target: ChangeAnalysisTarget;
  scope: ChangeAnalysisScope;
  siblings: { entries: NavSiblingEntry[]; index: number };
}) {
  const ref = changeAnalysisRef(target, scope);
  const { handlers } = useRouteDispatch(ref, { siblings });
  return (
    <button
      type="button"
      data-testid="change-analysis-pivot-row"
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      title={`Drill into ${rowKey}`}
      style={pivotRowButton}
    >
      <span style={{ minWidth: 100, fontWeight: 500 }}>{rowKey}</span>
      <span style={{ minWidth: 60, color: "var(--text-muted)" }}>
        {files} file{files === 1 ? "" : "s"}
      </span>
      <div style={barTrack}>
        <div style={{ ...barFill, width: `${(files / maxFiles) * 100}%` }} />
      </div>
      {showAddDel ? (
        <>
          <span style={addCol}>+{additions}</span>
          <span style={delCol}>−{deletions}</span>
        </>
      ) : null}
    </button>
  );
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600 };
const tab: React.CSSProperties = {
  padding: "4px 10px",
  background: "transparent",
  color: "var(--text-muted)",
  // Split into longhand so the active variant can swap borderColor
  // without React warning about shorthand/non-shorthand mixing on
  // rerender.
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
  flex: 1,
  height: 8,
  background: "var(--surface-app)",
  borderRadius: 4,
  overflow: "hidden",
};
const barFill: React.CSSProperties = {
  height: "100%",
  background: "var(--text-link, #2563eb)",
};
const addCol: React.CSSProperties = { color: "var(--text-success, #16a34a)", width: 56, textAlign: "right", fontSize: 12 };
const delCol: React.CSSProperties = { color: "var(--text-danger, #dc2626)", width: 56, textAlign: "right", fontSize: 12 };
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

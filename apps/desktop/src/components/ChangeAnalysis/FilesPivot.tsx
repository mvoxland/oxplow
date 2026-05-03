import { useMemo, useState } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import type { FilePivots } from "./analysisHelpers.js";

type PivotKey = "extension" | "directory" | "status";

interface FilesPivotProps {
  pivots: FilePivots;
  files: BranchChangeEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

export function FilesPivot({ pivots, files, onOpenFile }: FilesPivotProps) {
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

  const filteredFiles = useMemo(() => {
    return [...files].sort((a, b) => a.path.localeCompare(b.path));
  }, [files]);

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
          rows.map((row) => (
            <div
              key={row.key}
              data-testid="change-analysis-pivot-row"
              style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}
            >
              <span style={{ minWidth: 100, fontWeight: 500 }}>{row.key}</span>
              <span style={{ minWidth: 60, color: "var(--text-muted)" }}>
                {row.files} file{row.files === 1 ? "" : "s"}
              </span>
              <div style={barTrack}>
                <div style={{ ...barFill, width: `${(row.files / maxFiles) * 100}%` }} />
              </div>
              {active !== "status" ? (
                <>
                  <span style={addCol}>+{row.additions}</span>
                  <span style={delCol}>−{row.deletions}</span>
                </>
              ) : null}
            </div>
          ))
        )}
      </div>
      <details style={{ marginTop: 12 }}>
        <summary style={{ cursor: "pointer", fontSize: 12, color: "var(--text-muted)" }}>
          Show all {filteredFiles.length} file{filteredFiles.length === 1 ? "" : "s"}
        </summary>
        <div style={{ display: "flex", flexDirection: "column", marginTop: 6 }}>
          {filteredFiles.map((file) => (
            <button
              key={file.path}
              type="button"
              data-testid="change-analysis-file-row"
              onClick={(e) => onOpenFile(file.path, { newTab: e.metaKey || e.ctrlKey })}
              style={fileRow}
            >
              <span style={{ width: 16, color: "var(--text-muted)" }}>{statusBadge(file.status)}</span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {file.path}
              </span>
              <span style={addCol}>+{file.additions ?? 0}</span>
              <span style={delCol}>−{file.deletions ?? 0}</span>
            </button>
          ))}
        </div>
      </details>
    </section>
  );
}

function statusBadge(status: string): string {
  switch (status) {
    case "modified":
      return "M";
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "renamed":
      return "R";
    case "untracked":
      return "U";
    default:
      return "·";
  }
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
  border: "1px solid var(--border-subtle)",
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
const fileRow: React.CSSProperties = {
  display: "flex",
  gap: 8,
  alignItems: "center",
  padding: "2px 4px",
  fontSize: 12,
  background: "transparent",
  border: "none",
  textAlign: "left",
  cursor: "pointer",
  color: "var(--text-primary)",
};

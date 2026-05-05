import type { BranchChangeEntry } from "../../api-types.js";

interface FileChurnCardProps {
  files: BranchChangeEntry[];
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
}

const MAX_ROWS = 10;

/**
 * Top files by total line churn (`additions + deletions`). Each row
 * shows a stacked bar visualizing the add/delete split so reviewers
 * can see at a glance whether a file grew, shrank, or rewrote.
 *
 * Hides itself when nothing churns — a doc-only or no-op diff.
 */
export function FileChurnCard({ files, onOpenFile }: FileChurnCardProps) {
  const sorted = [...files]
    .map((f) => ({
      file: f,
      total: (f.additions ?? 0) + (f.deletions ?? 0),
    }))
    .filter((row) => row.total > 0)
    .sort((a, b) => b.total - a.total)
    .slice(0, MAX_ROWS);

  if (sorted.length === 0) return null;

  const max = sorted[0]!.total;

  return (
    <section data-testid="change-analysis-file-churn" style={card}>
      <div style={header}>Top files by line churn</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {sorted.map(({ file, total }) => {
          const adds = file.additions ?? 0;
          const dels = file.deletions ?? 0;
          const widthPct = (total / max) * 100;
          const addPct = total === 0 ? 0 : (adds / total) * 100;
          return (
            <div
              key={file.path}
              style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}
              data-testid="change-analysis-file-churn-row"
            >
              <button
                type="button"
                onClick={(e) => onOpenFile?.(file.path, { newTab: e.metaKey || e.ctrlKey })}
                style={pathButton}
                title={file.path}
              >
                {file.path}
              </button>
              <div
                style={{
                  flex: 1,
                  minWidth: 80,
                  height: 8,
                  background: "var(--surface-muted, var(--surface-card))",
                  borderRadius: 2,
                  overflow: "hidden",
                  display: "flex",
                  width: `${widthPct}%`,
                }}
              >
                <span
                  style={{
                    width: `${addPct}%`,
                    background: "var(--text-success, #16a34a)",
                  }}
                />
                <span
                  style={{
                    width: `${100 - addPct}%`,
                    background: "var(--text-danger, #dc2626)",
                  }}
                />
              </div>
              <span style={{ color: "var(--text-success, #16a34a)", minWidth: 40, textAlign: "right" }}>
                +{adds}
              </span>
              <span style={{ color: "var(--text-danger, #dc2626)", minWidth: 40, textAlign: "right" }}>
                −{dels}
              </span>
            </div>
          );
        })}
      </div>
    </section>
  );
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600, marginBottom: 8 };
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

import type { FunctionChurnRow, FunctionsBuckets } from "./analysisHelpers.js";

interface FunctionChurnCardProps {
  /** Per-function churn rows from `useChangeAnalysis.functionChurn`. */
  churn: FunctionChurnRow[];
  /** Used to look up complexityDelta for tiebreak. */
  functions: FunctionsBuckets;
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
}

const ROW_CAP = 15;
const MIN_CHURN_LINES = 5;
const MIN_CHURN_PCT = 0.05;

interface Row {
  key: string;
  path: string;
  name: string;
  containerPath: string[];
  startLine: number;
  added: number;
  deleted: number;
  churnPercent: number;
  complexityDelta: number;
}

/**
 * Functions ranked by share of total diff churn. A function appears
 * if it touches ≥5 lines OR contributes ≥5% of total diff churn.
 * Tiebreak by complexityDelta to surface "small but meaningful"
 * edits ahead of mechanical refactors.
 */
export function FunctionChurnCard({ churn, functions, onOpenFile }: FunctionChurnCardProps) {
  if (churn.length === 0) return null;

  const total = churn.reduce((acc, c) => acc + c.addedLines + c.deletedLines, 0);
  if (total === 0) return null;

  // Look up complexityDelta + startLine from the buckets when we
  // have it. Falls back to startLineHead from the churn row.
  const deltaLookup = new Map<string, { complexityDelta: number; startLine: number }>();
  for (const fn of functions.modifiedBody) {
    deltaLookup.set(qkey(fn.path, fn.containerPath, fn.name), {
      complexityDelta: fn.complexityDelta,
      startLine: fn.startLine,
    });
  }

  const rows: Row[] = churn
    .map((c) => {
      const meta = deltaLookup.get(qkey(c.path, c.containerPath, c.name));
      const sum = c.addedLines + c.deletedLines;
      return {
        key: qkey(c.path, c.containerPath, c.name),
        path: c.path,
        name: c.name,
        containerPath: c.containerPath,
        startLine: meta?.startLine ?? c.startLineHead,
        added: c.addedLines,
        deleted: c.deletedLines,
        churnPercent: total === 0 ? 0 : sum / total,
        complexityDelta: meta?.complexityDelta ?? 0,
      };
    })
    .filter((r) => r.added + r.deleted >= MIN_CHURN_LINES || r.churnPercent >= MIN_CHURN_PCT)
    .sort((a, b) => b.churnPercent - a.churnPercent || b.complexityDelta - a.complexityDelta)
    .slice(0, ROW_CAP);

  if (rows.length === 0) return null;

  return (
    <section data-testid="change-analysis-function-churn" style={card}>
      <div style={header}>Functions by churn</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {rows.map((row) => (
          <div
            key={row.key}
            style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}
            data-testid="change-analysis-function-churn-row"
          >
            <span
              style={{
                fontFamily: "ui-monospace, monospace",
                fontSize: 11,
                color: "var(--text-muted)",
                minWidth: 50,
                textAlign: "right",
              }}
              title={`${(row.churnPercent * 100).toFixed(1)}% of total diff churn`}
            >
              {(row.churnPercent * 100).toFixed(0)}%
            </span>
            <span style={{ color: "var(--text-success, #16a34a)", minWidth: 36, textAlign: "right" }}>
              +{row.added}
            </span>
            <span style={{ color: "var(--text-danger, #dc2626)", minWidth: 36, textAlign: "right" }}>
              −{row.deleted}
            </span>
            <span style={{ color: "var(--text-primary)", fontWeight: 600 }}>
              {row.containerPath.length > 0 ? `${row.containerPath.join("::")}::` : ""}
              {row.name}
            </span>
            {row.complexityDelta !== 0 ? (
              <span
                style={{
                  fontFamily: "ui-monospace, monospace",
                  fontSize: 11,
                  color: row.complexityDelta > 0 ? "var(--text-danger, #dc2626)" : "var(--text-muted)",
                }}
                title={`complexity ${row.complexityDelta > 0 ? "+" : ""}${row.complexityDelta}`}
              >
                cc {row.complexityDelta > 0 ? "+" : ""}{row.complexityDelta}
              </span>
            ) : null}
            <button
              type="button"
              onClick={(e) => onOpenFile?.(row.path, { newTab: e.metaKey || e.ctrlKey })}
              style={pathButton}
              title={row.path}
            >
              {row.path}:{row.startLine}
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}

function qkey(path: string, containerPath: string[], name: string): string {
  return containerPath.length === 0
    ? `${path}::${name}`
    : `${path}::${containerPath.join("::")}::${name}`;
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
  marginLeft: "auto",
};

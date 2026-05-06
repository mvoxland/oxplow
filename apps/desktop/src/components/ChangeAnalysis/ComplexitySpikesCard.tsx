import type { FunctionsBuckets } from "./analysisHelpers.js";

interface ComplexitySpikesCardProps {
  functions: FunctionsBuckets;
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
  /** Plain click on a row → diff in current tab at the function's
   *  start line. Cmd/ctrl-click → new-tab file open. */
  onOpenFileDiff?: (path: string, line?: number) => void;
}

const ROW_CAP = 12;
const DELTA_THRESHOLD = 2;
const ADDED_COMPLEXITY_THRESHOLD = 8;

interface SpikeRow {
  path: string;
  name: string;
  containerPath: string[];
  startLine: number;
  /** "+" delta for modifiedBody, absolute complexity prefixed with
   *  "new" for added functions. Used for sort + display. */
  badge: string;
  weight: number;
  kind: "delta" | "new";
}

/**
 * Functions whose complexity rose materially in this diff. Two
 * inputs:
 *
 *   - `modifiedBody` rows whose `complexityDelta >= 2`, sorted by
 *     delta desc.
 *   - `added` rows whose absolute `complexity >= 8` — newly-introduced
 *     hotspots aren't a "regression" but are still worth flagging.
 *
 * Hides itself when neither category fires.
 */
export function ComplexitySpikesCard({ functions, onOpenFile, onOpenFileDiff }: ComplexitySpikesCardProps) {
  const deltaRows: SpikeRow[] = functions.modifiedBody
    .filter((fn) => fn.complexityDelta >= DELTA_THRESHOLD)
    .map((fn) => ({
      path: fn.path,
      name: fn.name,
      containerPath: fn.containerPath,
      startLine: fn.startLine,
      badge: `+${fn.complexityDelta}`,
      weight: fn.complexityDelta,
      kind: "delta" as const,
    }));
  const newRows: SpikeRow[] = functions.added
    .filter((fn) => fn.complexity >= ADDED_COMPLEXITY_THRESHOLD)
    .map((fn) => ({
      path: fn.path,
      name: fn.name,
      containerPath: fn.containerPath,
      startLine: fn.startLine,
      badge: `new · cc ${fn.complexity}`,
      weight: fn.complexity,
      kind: "new" as const,
    }));
  const rows = [...deltaRows, ...newRows]
    .sort((a, b) => b.weight - a.weight)
    .slice(0, ROW_CAP);

  if (rows.length === 0) return null;

  return (
    <section data-testid="change-analysis-complexity-spikes" style={card}>
      <div style={header}>Complexity spikes</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {rows.map((row) => (
          <div
            key={`${row.path}::${row.containerPath.join("::")}::${row.name}`}
            style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}
            data-testid="change-analysis-complexity-row"
          >
            <span
              style={{
                fontFamily: "ui-monospace, monospace",
                fontSize: 11,
                color: row.kind === "new" ? "var(--text-muted)" : "var(--text-danger, #dc2626)",
                minWidth: 64,
              }}
            >
              {row.badge}
            </span>
            <span style={{ color: "var(--text-primary)", fontWeight: 600 }}>
              {row.containerPath.length > 0 ? `${row.containerPath.join("::")}::` : ""}
              {row.name}
            </span>
            <button
              type="button"
              onClick={(e) => {
                if (e.metaKey || e.ctrlKey) {
                  onOpenFile?.(row.path, { newTab: true });
                  return;
                }
                if (onOpenFileDiff) onOpenFileDiff(row.path, row.startLine);
                else onOpenFile?.(row.path);
              }}
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

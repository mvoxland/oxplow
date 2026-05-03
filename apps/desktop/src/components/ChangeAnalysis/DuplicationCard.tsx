import type { CodeQualityFindingRow } from "../../api-types.js";

interface DuplicationCardProps {
  duplication: {
    findings: CodeQualityFindingRow[];
    scanAgeMs: number | null;
    scanning: boolean;
    refresh(): Promise<void>;
  };
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

export function DuplicationCard({ duplication, onOpenFile }: DuplicationCardProps) {
  const dupes = duplication.findings.filter((f) => f.kind === "duplicate-block");
  return (
    <section data-testid="change-analysis-duplication" style={card}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
        <div style={header}>Duplication</div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
            {duplication.scanAgeMs == null
              ? "no jscpd scan yet"
              : `last scan: ${formatAge(duplication.scanAgeMs)} ago`}
          </span>
          <button
            type="button"
            data-testid="change-analysis-duplication-refresh"
            onClick={() => void duplication.refresh()}
            disabled={duplication.scanning}
            style={smallButton}
          >
            {duplication.scanning ? "Scanning…" : "Refresh"}
          </button>
        </div>
      </div>
      {dupes.length === 0 ? (
        <div style={muted}>
          No duplicate-block findings touch the changed files. Run a fresh jscpd scan if results
          look stale.
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {dupes.slice(0, 25).map((f) => {
            const peerPath = (f.extra?.peerPath as string | undefined) ?? null;
            const peerStart = (f.extra?.peerStartLine as number | undefined) ?? null;
            const peerEnd = (f.extra?.peerEndLine as number | undefined) ?? null;
            return (
              <div
                key={f.id}
                data-testid="change-analysis-duplicate-row"
                style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}
              >
                <button
                  type="button"
                  onClick={(e) => onOpenFile(f.path, { newTab: e.metaKey || e.ctrlKey })}
                  style={pathButton}
                >
                  {f.path}:{f.startLine}-{f.endLine}
                </button>
                <span style={{ color: "var(--text-muted)" }}>↔</span>
                {peerPath ? (
                  <button
                    type="button"
                    onClick={(e) => onOpenFile(peerPath, { newTab: e.metaKey || e.ctrlKey })}
                    style={pathButton}
                  >
                    {peerPath}
                    {peerStart != null && peerEnd != null ? `:${peerStart}-${peerEnd}` : ""}
                  </button>
                ) : (
                  <span style={muted}>(unknown peer)</span>
                )}
                <span style={{ marginLeft: "auto", color: "var(--text-muted)" }}>
                  {f.metricValue} lines
                </span>
              </div>
            );
          })}
          {dupes.length > 25 ? (
            <div style={muted}>…and {dupes.length - 25} more</div>
          ) : null}
        </div>
      )}
    </section>
  );
}

function formatAge(ms: number): string {
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 48) return `${hr}h`;
  const days = Math.floor(hr / 24);
  return `${days}d`;
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600 };
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
const smallButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 12,
};
const pathButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: 12,
};

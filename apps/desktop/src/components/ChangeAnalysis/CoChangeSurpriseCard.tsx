import { useMemo } from "react";
import type { FileSurprise } from "../../tauri-bridge/index.js";

interface Props {
  surprise: FileSurprise[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Renders the behavioural answer to "weird that this was touched."
 * For each file in the diff, the backend
 * (`analyze_co_change_surprise`) returns one of three reasons:
 *
 * - `Normal` — co-changers present, or no historical signal worth
 *   surfacing. Omitted from this view.
 * - `UsualCoChangersAbsent { expected }` — file usually moves with
 *   X / Y / Z but those aren't in this commit.
 * - `Dormant { last_touched_days }` — file hasn't been touched in
 *   ≥ 90 days (configured floor).
 *
 * The card hides itself entirely when nothing is surprising.
 */
export function CoChangeSurpriseCard({ surprise, onOpenFile }: Props) {
  const flagged = useMemo(
    () => surprise.filter((s) => s.reason.kind !== "normal"),
    [surprise],
  );
  if (flagged.length === 0) return null;

  const dormantCount = flagged.filter((s) => s.reason.kind === "dormant").length;
  const lonelyCount = flagged.filter((s) => s.reason.kind === "usual_co_changers_absent").length;

  return (
    <div style={card}>
      <header style={cardHeader}>
        <h3 style={cardTitle}>Co-change surprises</h3>
        <span style={muted}>
          {[
            dormantCount ? `${dormantCount} dormant` : null,
            lonelyCount ? `${lonelyCount} usually-paired touched alone` : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </header>
      <ul style={list}>
        {flagged.map((entry) => (
          <li key={entry.path} style={row}>
            <button
              type="button"
              style={pathBtn}
              onClick={(e) =>
                onOpenFile(entry.path, { newTab: e.metaKey || e.ctrlKey })
              }
              title={`Open ${entry.path}`}
            >
              {entry.path}
            </button>
            <SurpriseChip reason={entry.reason} />
          </li>
        ))}
      </ul>
    </div>
  );
}

function SurpriseChip({ reason }: { reason: FileSurprise["reason"] }) {
  if (reason.kind === "dormant") {
    return (
      <span
        style={{ ...chip, background: "#fef3c7", color: "#92400e" }}
        title={`Hasn't been touched in ${reason.last_touched_days} days`}
      >
        dormant · {reason.last_touched_days}d
      </span>
    );
  }
  if (reason.kind === "usual_co_changers_absent") {
    const list = reason.expected.slice(0, 3).join(", ");
    const more =
      reason.expected.length > 3 ? ` (+${reason.expected.length - 3} more)` : "";
    return (
      <span
        style={{ ...chip, background: "#dbeafe", color: "#1e40af" }}
        title={`Usually touched with: ${reason.expected.join(", ")}`}
      >
        usually with: {list}
        {more}
      </span>
    );
  }
  return null;
}

const card: React.CSSProperties = {
  border: "1px solid var(--border, #e5e5e5)",
  borderRadius: 6,
  padding: 12,
  background: "var(--surface, #fff)",
  display: "flex",
  flexDirection: "column",
  gap: 8,
};
const cardHeader: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "baseline",
};
const cardTitle: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--text-base, 14px)",
  fontWeight: 600,
};
const muted: React.CSSProperties = {
  color: "var(--text-muted, #737373)",
  fontSize: 11,
};
const list: React.CSSProperties = {
  listStyle: "none",
  padding: 0,
  margin: 0,
  display: "flex",
  flexDirection: "column",
  gap: 4,
};
const row: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  fontSize: 12,
};
const pathBtn: React.CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  fontFamily: "var(--font-mono, monospace)",
  fontSize: 12,
  color: "var(--text-primary, #111)",
  cursor: "pointer",
  textAlign: "left",
  flex: 1,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
const chip: React.CSSProperties = {
  padding: "1px 6px",
  borderRadius: 3,
  fontSize: 10,
  fontWeight: 500,
  whiteSpace: "nowrap",
};

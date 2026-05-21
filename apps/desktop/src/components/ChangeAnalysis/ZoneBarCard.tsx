import { useMemo } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import type { ImportDelta } from "../../tauri-bridge/index.js";
import { classifyZone, ZONE_COLORS, ZONE_LABELS, type Zone } from "./zones.js";

interface Props {
  files: BranchChangeEntry[];
  importDeltas: ImportDelta[];
}

/**
 * Architectural-zone summary for a diff: which zones did this commit
 * touch, and did any *new* import edges cross a zone boundary? The
 * top bar shows each touched zone as a cell whose width is
 * proportional to that zone's churn (additions + deletions). The
 * callouts below list every cross-zone import the diff introduced —
 * "this UI commit now reaches into store" is the headline signal.
 *
 * Pure-deterministic: zone assignment comes from
 * `apps/desktop/src/components/ChangeAnalysis/zones.ts`, which
 * mirrors the Rust table in `crates/oxplow-code-deps/src/zones.rs`.
 */
export function ZoneBarCard({ files, importDeltas }: Props) {
  const buckets = useMemo(() => bucketByZone(files), [files]);
  const crossZone = useMemo(() => collectCrossZoneEdges(importDeltas), [importDeltas]);

  if (buckets.length === 0) {
    return null;
  }
  const total = buckets.reduce((acc, b) => acc + b.churn, 0) || 1;

  return (
    <div style={card}>
      <header style={cardHeader}>
        <h3 style={cardTitle}>Architectural zones</h3>
        <span style={muted}>
          {buckets.length} zone{buckets.length === 1 ? "" : "s"} touched
        </span>
      </header>

      <div style={bar} role="img" aria-label="Zones touched, sized by churn">
        {buckets.map((b) => {
          const pct = (b.churn / total) * 100;
          return (
            <div
              key={b.zone}
              style={{
                ...barCell,
                background: zoneColor(b.zone),
                flexBasis: `${pct}%`,
                minWidth: 32,
              }}
              title={`${ZONE_LABELS[b.zone]} — ${b.fileCount} file${b.fileCount === 1 ? "" : "s"}, ±${b.churn} lines`}
            >
              <span style={barCellLabel}>{ZONE_LABELS[b.zone]}</span>
              <span style={barCellCount}>{b.fileCount}</span>
            </div>
          );
        })}
      </div>

      {crossZone.length > 0 ? (
        <div style={crossZoneSection}>
          <div style={crossZoneHeader}>
            <strong>Cross-zone imports added ({crossZone.length})</strong>
            <span style={muted}>
              new edges that cross architectural boundaries
            </span>
          </div>
          <ul style={crossZoneList}>
            {crossZone.slice(0, 12).map((edge, i) => (
              <li key={`${edge.fromPath}:${edge.module}:${i}`} style={crossZoneRow}>
                <span style={zoneChip(edge.fromZone)}>
                  {ZONE_LABELS[edge.fromZone]}
                </span>
                <span style={arrow}>→</span>
                <span style={zoneChip(edge.toZone)}>{ZONE_LABELS[edge.toZone]}</span>
                <code style={moduleCode}>{edge.module}</code>
                <span style={muted}>in {shortPath(edge.fromPath)}</span>
              </li>
            ))}
            {crossZone.length > 12 ? (
              <li style={{ ...crossZoneRow, color: "var(--text-muted)" }}>
                …and {crossZone.length - 12} more
              </li>
            ) : null}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

interface ZoneBucket {
  zone: Zone;
  fileCount: number;
  churn: number;
}

function bucketByZone(files: BranchChangeEntry[]): ZoneBucket[] {
  const map = new Map<Zone, ZoneBucket>();
  for (const f of files) {
    const z = classifyZone(f.path);
    const entry = map.get(z) ?? { zone: z, fileCount: 0, churn: 0 };
    entry.fileCount += 1;
    entry.churn += (f.additions ?? 0) + (f.deletions ?? 0);
    map.set(z, entry);
  }
  return [...map.values()].sort((a, b) => b.churn - a.churn);
}

interface CrossZoneEdge {
  fromPath: string;
  module: string;
  fromZone: Zone;
  toZone: Zone;
}

function collectCrossZoneEdges(deltas: ImportDelta[]): CrossZoneEdge[] {
  const out: CrossZoneEdge[] = [];
  for (const d of deltas) {
    for (const zedge of d.cross_zone_added ?? []) {
      // External targets are filtered server-side, but be defensive.
      if (!zedge.to_zone || zedge.to_zone === "external") continue;
      if (zedge.to_zone === zedge.from_zone) continue;
      out.push({
        fromPath: d.path,
        module: zedge.edge.module,
        fromZone: zedge.from_zone as Zone,
        toZone: zedge.to_zone as Zone,
      });
    }
  }
  return out;
}

function shortPath(p: string): string {
  // Strip leading `crates/` or `apps/desktop/src/` for compactness.
  return p
    .replace(/^crates\//, "")
    .replace(/^apps\/desktop\/src\//, "ui/")
    .replace(/^apps\/desktop\/src-tauri\//, "shell/");
}

function zoneColor(z: Zone): string {
  return ZONE_COLORS[z];
}

function zoneChip(z: Zone): React.CSSProperties {
  return {
    background: zoneColor(z),
    color: "white",
    padding: "1px 6px",
    borderRadius: 3,
    fontSize: 11,
    fontWeight: 500,
    fontFamily: "var(--font-mono, monospace)",
    lineHeight: 1.4,
  };
}

const card: React.CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  display: "flex",
  flexDirection: "column",
  gap: 10,
};
const cardHeader: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
};
const cardTitle: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--text-base, 14px)",
  fontWeight: 600,
  color: "var(--text-primary)",
};
const muted: React.CSSProperties = {
  color: "var(--text-secondary)",
  fontSize: 11,
};
const bar: React.CSSProperties = {
  display: "flex",
  width: "100%",
  height: 28,
  borderRadius: 4,
  overflow: "hidden",
  border: "1px solid var(--border-subtle)",
};
const barCell: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  gap: 6,
  color: "white",
  fontSize: 11,
  fontWeight: 500,
  padding: "0 8px",
  whiteSpace: "nowrap",
  overflow: "hidden",
};
const barCellLabel: React.CSSProperties = {
  overflow: "hidden",
  textOverflow: "ellipsis",
};
const barCellCount: React.CSSProperties = {
  background: "rgba(0,0,0,0.25)",
  borderRadius: 8,
  padding: "0 6px",
  fontSize: 10,
};
const crossZoneSection: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 6,
  paddingTop: 6,
  borderTop: "1px dashed var(--border-subtle)",
};
const crossZoneHeader: React.CSSProperties = {
  display: "flex",
  gap: 8,
  alignItems: "baseline",
};
const crossZoneList: React.CSSProperties = {
  listStyle: "none",
  padding: 0,
  margin: 0,
  display: "flex",
  flexDirection: "column",
  gap: 4,
};
const crossZoneRow: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  fontSize: 12,
};
const arrow: React.CSSProperties = {
  color: "var(--text-secondary)",
};
const moduleCode: React.CSSProperties = {
  fontFamily: "var(--font-mono, monospace)",
  fontSize: 11,
  // Lift above the card surface so the chip reads cleanly.
  background: "var(--surface-elevated)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  padding: "1px 4px",
  borderRadius: 3,
};

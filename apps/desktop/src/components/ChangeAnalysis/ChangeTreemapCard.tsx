import { useMemo, useRef, useState, useEffect } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import { classifyZone, ZONE_LABELS, type Zone } from "./zones.js";

interface Props {
  files: BranchChangeEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Squarified treemap of the commit's file churn. One rectangle per
 * file, area proportional to `additions + deletions`, colour by
 * architectural zone (same palette as ZoneBarCard). Renders inline
 * SVG sized to the container width — no external graph library
 * dependency.
 *
 * The visual gestalt answers "where is this commit's mass?" at a
 * glance: a UI-only commit and a UI+store+migration commit look
 * obviously different.
 */
export function ChangeTreemapCard({ files, onOpenFile }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(640);
  useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) {
        const w = Math.max(120, e.contentRect.width);
        setContainerWidth(w);
      }
    });
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);

  const cells = useMemo(
    () => layoutTreemap(files, containerWidth, 240),
    [files, containerWidth],
  );

  if (cells.length === 0) {
    return null;
  }

  return (
    <div style={card} ref={ref}>
      <header style={cardHeader}>
        <h3 style={cardTitle}>Change treemap</h3>
        <span style={muted}>
          rectangle area ∝ churn, colour ∝ zone
        </span>
      </header>
      <svg
        width={containerWidth}
        height={240}
        style={{ display: "block" }}
        role="img"
        aria-label="Treemap of files by churn and zone"
      >
        {cells.map((cell) => (
          <g key={cell.file.path}>
            <rect
              x={cell.x}
              y={cell.y}
              width={cell.w}
              height={cell.h}
              fill={ZONE_COLORS[cell.zone]}
              stroke="white"
              strokeWidth={1}
              onClick={(e) =>
                onOpenFile(cell.file.path, { newTab: e.metaKey || e.ctrlKey })
              }
              style={{ cursor: "pointer" }}
            >
              <title>
                {cell.file.path} ({ZONE_LABELS[cell.zone]}) · +
                {cell.file.additions ?? 0} −{cell.file.deletions ?? 0}
              </title>
            </rect>
            {cell.w > 60 && cell.h > 20 ? (
              <text
                x={cell.x + 4}
                y={cell.y + 14}
                fontSize={11}
                fill="white"
                pointerEvents="none"
                style={{ fontFamily: "var(--font-mono, monospace)" }}
              >
                {truncate(basename(cell.file.path), Math.floor(cell.w / 7))}
              </text>
            ) : null}
          </g>
        ))}
      </svg>
    </div>
  );
}

interface TreemapCell {
  file: BranchChangeEntry;
  zone: Zone;
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap (Bruls, Huijsen, Van Wijk 2000). Sorts files
 * by churn desc, then greedily lays out rows that minimize the worst
 * aspect ratio.
 */
function layoutTreemap(
  files: BranchChangeEntry[],
  width: number,
  height: number,
): TreemapCell[] {
  if (files.length === 0 || width <= 0 || height <= 0) return [];
  type Item = { file: BranchChangeEntry; value: number };
  const items: Item[] = files
    .map((f) => ({
      file: f,
      value: Math.max(1, (f.additions ?? 0) + (f.deletions ?? 0)),
    }))
    .sort((a, b) => b.value - a.value);

  const totalValue = items.reduce((acc, i) => acc + i.value, 0);
  const totalArea = width * height;
  // Scale every value into pixel-area.
  const scaled = items.map((i) => ({
    file: i.file,
    area: (i.value / totalValue) * totalArea,
  }));

  const cells: TreemapCell[] = [];
  let x = 0;
  let y = 0;
  let w = width;
  let h = height;
  let queue = [...scaled];

  while (queue.length > 0) {
    const shorter = Math.min(w, h);
    // Greedily extend the row while aspect ratio improves.
    const row: typeof queue = [queue[0]!];
    queue = queue.slice(1);
    while (queue.length > 0) {
      const candidate = [...row, queue[0]!];
      if (worstRatio(candidate, shorter) <= worstRatio(row, shorter)) {
        row.push(queue[0]!);
        queue = queue.slice(1);
      } else {
        break;
      }
    }
    // Lay out the row along the shorter side.
    const rowTotal = row.reduce((acc, r) => acc + r.area, 0);
    const rowExtent = rowTotal / shorter;
    if (w >= h) {
      // shorter is height; row sits along the LEFT edge, stacked vertically.
      let cy = y;
      for (const r of row) {
        const cellH = r.area / rowExtent;
        cells.push({
          file: r.file,
          zone: classifyZone(r.file.path),
          x,
          y: cy,
          w: rowExtent,
          h: cellH,
        });
        cy += cellH;
      }
      x += rowExtent;
      w -= rowExtent;
    } else {
      // shorter is width; row sits along the TOP edge, laid horizontally.
      let cx = x;
      for (const r of row) {
        const cellW = r.area / rowExtent;
        cells.push({
          file: r.file,
          zone: classifyZone(r.file.path),
          x: cx,
          y,
          w: cellW,
          h: rowExtent,
        });
        cx += cellW;
      }
      y += rowExtent;
      h -= rowExtent;
    }
  }
  return cells;
}

/** Worst aspect ratio if `row` is laid along edge of length `shorter`. */
function worstRatio(row: { area: number }[], shorter: number): number {
  const s = row.reduce((acc, r) => acc + r.area, 0);
  if (s === 0) return Number.POSITIVE_INFINITY;
  let max = 0;
  let min = Number.POSITIVE_INFINITY;
  for (const r of row) {
    if (r.area > max) max = r.area;
    if (r.area < min) min = r.area;
  }
  const ss = s * s;
  const w2 = shorter * shorter;
  return Math.max((w2 * max) / ss, ss / (w2 * min));
}

function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return i === -1 ? p : p.slice(i + 1);
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  if (max <= 1) return s.slice(0, max);
  return s.slice(0, max - 1) + "…";
}

// Mirrors the palette in ZoneBarCard. Kept in sync by hand for now —
// promotable to a shared module if a third consumer appears.
const ZONE_COLORS: Record<Zone, string> = {
  ui: "#4f46e5",
  shell: "#0ea5e9",
  ipc: "#06b6d4",
  domain: "#0891b2",
  store: "#ea580c",
  git: "#dc2626",
  lsp: "#ca8a04",
  runtime: "#9333ea",
  fs_watch: "#a16207",
  terminal: "#525252",
  mcp: "#16a34a",
  app_orchestration: "#2563eb",
  config: "#737373",
  session: "#7c3aed",
  plugin: "#c026d3",
  analysis: "#0d9488",
  migration: "#b91c1c",
  test: "#22c55e",
  docs: "#a3a3a3",
  project_meta: "#64748b",
  external: "#6b7280",
  other: "#94a3b8",
};

const card: React.CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  background: "var(--surface-card)",
  color: "var(--text-primary)",
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
  color: "var(--text-primary)",
};
const muted: React.CSSProperties = {
  color: "var(--text-muted)",
  fontSize: 11,
};

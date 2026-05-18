import { useMemo, useRef, useState, useEffect } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import { classifyZone, ZONE_LABELS, type Zone } from "./zones.js";

interface Props {
  files: BranchChangeEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Squarified treemap of the commit's file churn, grouped by
 * architectural zone (WinDirStat-style). Each touched zone gets
 * one contiguous block sized by its total churn; files within the
 * zone are laid out as a sub-treemap inside that block sharing the
 * zone colour. A header band labels each zone group when the rect
 * is large enough to fit it.
 *
 * Renders inline SVG sized to the container width — no external
 * graph library dependency.
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

  const layout = useMemo(
    () => layoutTreemapByZone(files, containerWidth, 240),
    [files, containerWidth],
  );

  if (layout.cells.length === 0) {
    return null;
  }

  return (
    <div style={card} ref={ref}>
      <header style={cardHeader}>
        <h3 style={cardTitle}>Change treemap</h3>
        <span style={muted}>
          grouped by zone · area ∝ churn
        </span>
      </header>
      <svg
        width={containerWidth}
        height={240}
        style={{ display: "block" }}
        role="img"
        aria-label="Treemap of files by churn, grouped by architectural zone"
      >
        {layout.zones.map((z) => (
          <g key={`zone-${z.zone}`} pointerEvents="none">
            {z.headerVisible ? (
              <>
                <rect
                  x={z.x}
                  y={z.y}
                  width={z.w}
                  height={HEADER_HEIGHT}
                  fill="rgba(0,0,0,0.35)"
                />
                <text
                  x={z.x + 6}
                  y={z.y + 11}
                  fontSize={10}
                  fontWeight={600}
                  fill="white"
                  style={{ fontFamily: "var(--font-mono, monospace)" }}
                >
                  {truncate(
                    `${ZONE_LABELS[z.zone]} · ${z.fileCount} file${z.fileCount === 1 ? "" : "s"} · ±${z.churn}`,
                    Math.floor(z.w / 6),
                  )}
                </text>
              </>
            ) : null}
          </g>
        ))}
        {layout.cells.map((cell) => (
          <g key={cell.file.path}>
            <rect
              x={cell.x}
              y={cell.y}
              width={cell.w}
              height={cell.h}
              fill={ZONE_COLORS[cell.zone]}
              stroke="var(--surface-card)"
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

interface ZoneGroup {
  zone: Zone;
  /** Total file count in the zone — included in the header band label. */
  fileCount: number;
  /** Total churn (additions + deletions) for the whole zone. */
  churn: number;
  x: number;
  y: number;
  w: number;
  h: number;
  /** True when the zone rect is big enough to fit a readable header band
   *  without clipping. Determines whether the band is drawn AND whether
   *  the inner file layout reserves space for it. */
  headerVisible: boolean;
}

/** Height of the zone-label band, when present. */
const HEADER_HEIGHT = 14;
/** Minimum rect dimensions before we'll render a header band. Below
 *  this, the band would clip or eat most of the zone's area. */
const HEADER_MIN_HEIGHT = 36;
const HEADER_MIN_WIDTH = 60;

/**
 * Two-level squarified treemap: outer pass packs zones by total
 * churn, inner pass packs each zone's files into its rect. Returns
 * both the zone group descriptors (for header bands) and the file
 * cells (for the actual fill rects).
 */
function layoutTreemapByZone(
  files: BranchChangeEntry[],
  width: number,
  height: number,
): { zones: ZoneGroup[]; cells: TreemapCell[] } {
  if (files.length === 0 || width <= 0 || height <= 0) {
    return { zones: [], cells: [] };
  }

  // Bucket files by zone, accumulating churn. Floor each file at 1
  // so rename-only / binary files still take a sliver of space.
  type Bucket = { zone: Zone; files: BranchChangeEntry[]; churn: number };
  const bucketMap = new Map<Zone, Bucket>();
  for (const f of files) {
    const z = classifyZone(f.path);
    const churn = Math.max(1, (f.additions ?? 0) + (f.deletions ?? 0));
    const entry = bucketMap.get(z) ?? { zone: z, files: [], churn: 0 };
    entry.files.push(f);
    entry.churn += churn;
    bucketMap.set(z, entry);
  }
  const buckets = [...bucketMap.values()].sort((a, b) => b.churn - a.churn);

  // Outer pass: each item is a zone, value = zone churn.
  const zoneRects = squarify(
    buckets.map((b) => ({ value: b.churn, payload: b })),
    0,
    0,
    width,
    height,
  );

  const zones: ZoneGroup[] = [];
  const cells: TreemapCell[] = [];
  for (const zr of zoneRects) {
    const b = zr.payload;
    const headerVisible =
      zr.h >= HEADER_MIN_HEIGHT && zr.w >= HEADER_MIN_WIDTH;
    const innerY = zr.y + (headerVisible ? HEADER_HEIGHT : 0);
    const innerH = zr.h - (headerVisible ? HEADER_HEIGHT : 0);
    zones.push({
      zone: b.zone,
      fileCount: b.files.length,
      churn: b.churn,
      x: zr.x,
      y: zr.y,
      w: zr.w,
      h: zr.h,
      headerVisible,
    });

    if (innerH <= 0 || zr.w <= 0) continue;

    // Inner pass: each item is a file, value = per-file churn.
    const fileRects = squarify(
      b.files.map((f) => ({
        value: Math.max(1, (f.additions ?? 0) + (f.deletions ?? 0)),
        payload: f,
      })),
      zr.x,
      innerY,
      zr.w,
      innerH,
    );
    for (const fr of fileRects) {
      cells.push({
        file: fr.payload,
        zone: b.zone,
        x: fr.x,
        y: fr.y,
        w: fr.w,
        h: fr.h,
      });
    }
  }

  return { zones, cells };
}

interface SquarifyInput<T> {
  value: number;
  payload: T;
}
interface SquarifyOutput<T> {
  payload: T;
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap (Bruls, Huijsen, Van Wijk 2000) generalized
 * to lay out into any rectangle. Sorts items by value desc, then
 * greedily packs rows whose worst aspect ratio doesn't degrade.
 */
function squarify<T>(
  items: SquarifyInput<T>[],
  x0: number,
  y0: number,
  width: number,
  height: number,
): SquarifyOutput<T>[] {
  if (items.length === 0 || width <= 0 || height <= 0) return [];
  const sorted = [...items].sort((a, b) => b.value - a.value);
  const totalValue = sorted.reduce((acc, i) => acc + i.value, 0);
  if (totalValue <= 0) return [];
  const totalArea = width * height;
  const scaled = sorted.map((i) => ({
    payload: i.payload,
    area: (i.value / totalValue) * totalArea,
  }));

  const out: SquarifyOutput<T>[] = [];
  let x = x0;
  let y = y0;
  let w = width;
  let h = height;
  let queue = scaled;

  while (queue.length > 0) {
    const shorter = Math.min(w, h);
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
    const rowTotal = row.reduce((acc, r) => acc + r.area, 0);
    const rowExtent = rowTotal / shorter;
    if (w >= h) {
      let cy = y;
      for (const r of row) {
        const cellH = r.area / rowExtent;
        out.push({ payload: r.payload, x, y: cy, w: rowExtent, h: cellH });
        cy += cellH;
      }
      x += rowExtent;
      w -= rowExtent;
    } else {
      let cx = x;
      for (const r of row) {
        const cellW = r.area / rowExtent;
        out.push({ payload: r.payload, x: cx, y, w: cellW, h: rowExtent });
        cx += cellW;
      }
      y += rowExtent;
      h -= rowExtent;
    }
  }
  return out;
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
  color: "var(--text-secondary)",
  fontSize: 11,
};

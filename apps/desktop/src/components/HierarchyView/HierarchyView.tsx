import { useMemo, useState } from "react";
import type { ReactNode } from "react";

export type HierarchyStatus = "added" | "modified" | "deleted";

export interface HierarchyNode {
  /** Stable id for keying + collapse-state lookups. Must be unique
   *  across the entire tree (callers usually prefix with the parent
   *  id to ensure uniqueness). */
  id: string;
  /** Visible label. Search matches against this. */
  label: string;
  /** Optional left-side icon (folder, file, fn, etc.). Should render
   *  at the same line-height as the label so it doesn't force the
   *  row taller — the wrapper sets `width: 1em; height: 1em`. */
  icon?: ReactNode;
  /** Status badges shown after the icon. Branch nodes pass the union
   *  of their descendants' statuses. */
  statuses?: Set<HierarchyStatus>;
  /** Muted text rendered after the label. */
  detail?: string;
  /** Numeric count rendered in muted parens after the detail. Branch
   *  nodes typically pass their descendant count. */
  count?: number;
  /** When supplied, the label area becomes a clickable button. */
  onDrill?(e: React.MouseEvent): void;
  /** Hover-title for the drill button. */
  drillTitle?: string;
  /** Test-id for the row. */
  testId?: string;
  children: HierarchyNode[];
}

export interface HierarchyViewProps {
  nodes: HierarchyNode[];
  /** Toolbar visibility. Defaults to true. */
  showToolbar?: boolean;
  searchPlaceholder?: string;
  /** Optional secondary content rendered at the right side of the
   *  toolbar (e.g. "N of M files" indicator). */
  toolbarExtra?: ReactNode;
  /** Test-id prefix for inputs / buttons. Defaults to `hierarchy`. */
  testIdPrefix?: string;
  /** Empty-state message when `nodes` is empty (or after a search
   *  filter prunes everything). */
  emptyLabel?: string;
}

/**
 * Generic hierarchical viewer used by Change Analysis's File-list and
 * Semantic views. Owns the toolbar (filter + Expand-all + Collapse-
 * all), the chevron toggle, and the status-badge rendering. Callers
 * shape their data into `HierarchyNode[]` once and the rest is
 * uniform.
 */
export function HierarchyView({
  nodes,
  showToolbar = true,
  searchPlaceholder = "Filter…",
  toolbarExtra,
  testIdPrefix = "hierarchy",
  emptyLabel = "Nothing to show.",
}: HierarchyViewProps) {
  const [search, setSearch] = useState("");
  // Collapse-state is stored as a Set of node ids the user has
  // explicitly collapsed; everything else is expanded by default.
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const filtered = useMemo(() => {
    if (!search.trim()) return nodes;
    const needle = search.toLowerCase();
    return nodes.map((n) => filterTree(n, needle)).filter((n): n is HierarchyNode => n != null);
  }, [nodes, search]);

  const allIds = useMemo(() => collectAllIds(filtered), [filtered]);

  const toggle = (id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const expandAll = () => setCollapsed(new Set());
  const collapseAll = () => setCollapsed(new Set(allIds));

  // While a search is active, force-expand so matches are visible.
  const effectivelyCollapsed = search.trim() ? new Set<string>() : collapsed;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {showToolbar ? (
        <div style={toolbarRow}>
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={searchPlaceholder}
            data-testid={`${testIdPrefix}-search`}
            style={searchInput}
          />
          <button
            type="button"
            data-testid={`${testIdPrefix}-expand-all`}
            onClick={expandAll}
            disabled={!!search.trim()}
            title={search.trim() ? "Search forces all rows expanded" : "Expand all branches"}
            style={smallButton}
          >
            Expand all
          </button>
          <button
            type="button"
            data-testid={`${testIdPrefix}-collapse-all`}
            onClick={collapseAll}
            disabled={!!search.trim()}
            title={search.trim() ? "Search forces all rows expanded" : "Collapse all branches"}
            style={smallButton}
          >
            Collapse all
          </button>
          {toolbarExtra ? <div style={{ marginLeft: "auto" }}>{toolbarExtra}</div> : null}
        </div>
      ) : null}
      {filtered.length === 0 ? (
        <div style={emptyStyle}>{emptyLabel}</div>
      ) : (
        <div data-testid={`${testIdPrefix}-tree`} style={{ display: "flex", flexDirection: "column" }}>
          {filtered.map((node) => (
            <Branch
              key={node.id}
              node={node}
              depth={0}
              collapsed={effectivelyCollapsed}
              onToggle={toggle}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function Branch({
  node,
  depth,
  collapsed,
  onToggle,
}: {
  node: HierarchyNode;
  depth: number;
  collapsed: Set<string>;
  onToggle(id: string): void;
}) {
  const isLeaf = node.children.length === 0;
  const expanded = !collapsed.has(node.id);
  return (
    <>
      <div data-testid={node.testId} style={branchRow(depth)}>
        {isLeaf ? (
          <span style={chevronSpacer} aria-hidden />
        ) : (
          <ChevronToggle expanded={expanded} onClick={() => onToggle(node.id)} />
        )}
        {node.icon ? <span style={iconWrapper}>{node.icon}</span> : null}
        {node.statuses && node.statuses.size > 0 ? (
          <StatusBadges statuses={node.statuses} />
        ) : null}
        {node.onDrill ? (
          <button
            type="button"
            onClick={node.onDrill}
            title={node.drillTitle}
            style={labelButton}
          >
            {node.label}
          </button>
        ) : (
          <span style={labelText}>{node.label}</span>
        )}
        {node.detail ? <span style={detailText}>{node.detail}</span> : null}
        {typeof node.count === "number" ? (
          <span style={countText}>({node.count})</span>
        ) : null}
      </div>
      {!isLeaf && expanded
        ? node.children.map((child) => (
            <Branch
              key={child.id}
              node={child}
              depth={depth + 1}
              collapsed={collapsed}
              onToggle={onToggle}
            />
          ))
        : null}
    </>
  );
}

function ChevronToggle({ expanded, onClick }: { expanded: boolean; onClick(): void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-expanded={expanded}
      aria-label={expanded ? "Collapse" : "Expand"}
      title={expanded ? "Collapse" : "Expand"}
      style={chevronButton}
    >
      <svg
        width="0.75em"
        height="0.75em"
        viewBox="0 0 10 10"
        aria-hidden
        style={{
          transform: expanded ? "rotate(90deg)" : "rotate(0deg)",
          transition: "transform 120ms ease",
          display: "block",
        }}
      >
        <path
          d="M3 1.5 L7 5 L3 8.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </button>
  );
}

export function StatusBadges({ statuses }: { statuses: Set<HierarchyStatus> }) {
  const order: HierarchyStatus[] = ["added", "modified", "deleted"];
  return (
    <span style={badgeRow}>
      {order
        .filter((s) => statuses.has(s))
        .map((s) => (
          <StatusBadge key={s} status={s} />
        ))}
    </span>
  );
}

export function StatusBadge({ status }: { status: HierarchyStatus }) {
  const cfg = STATUS_STYLE[status];
  return (
    <span title={cfg.title} data-testid={`hierarchy-status-${status}`} style={badgeStyle(cfg)}>
      {cfg.glyph}
    </span>
  );
}

const STATUS_STYLE: Record<HierarchyStatus, { glyph: string; fg: string; bg: string; title: string }> = {
  added: {
    glyph: "A",
    fg: "var(--text-success, #16a34a)",
    bg: "rgba(22, 163, 74, 0.18)",
    title: "Added",
  },
  modified: {
    glyph: "M",
    fg: "var(--text-link, #2563eb)",
    bg: "rgba(37, 99, 235, 0.18)",
    title: "Modified",
  },
  deleted: {
    glyph: "D",
    fg: "var(--text-danger, #dc2626)",
    bg: "rgba(220, 38, 38, 0.18)",
    title: "Deleted",
  },
};

/** Filter the tree to nodes whose label matches `needle` plus their
 *  ancestors. Returns null when neither the node nor any descendant
 *  matches. */
function filterTree(node: HierarchyNode, needle: string): HierarchyNode | null {
  const selfMatch = node.label.toLowerCase().includes(needle);
  const matchedChildren = node.children
    .map((c) => filterTree(c, needle))
    .filter((c): c is HierarchyNode => c != null);
  if (!selfMatch && matchedChildren.length === 0) return null;
  return { ...node, children: matchedChildren };
}

function collectAllIds(nodes: HierarchyNode[], out: string[] = []): string[] {
  for (const n of nodes) {
    if (n.children.length > 0) {
      out.push(n.id);
      collectAllIds(n.children, out);
    }
  }
  return out;
}

const CHEVRON_GUTTER = 16;
const ROW_FONT_SIZE = 12;
const toolbarRow: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
};
const searchInput: React.CSSProperties = {
  flex: 1,
  minWidth: 120,
  maxWidth: 320,
  padding: "4px 8px",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  fontSize: 12,
};
const smallButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 12,
};
const emptyStyle: React.CSSProperties = {
  color: "var(--text-muted)",
  fontSize: 12,
  padding: 8,
};
const branchRow = (depth: number): React.CSSProperties => ({
  display: "flex",
  alignItems: "center",
  gap: 4,
  fontSize: ROW_FONT_SIZE,
  lineHeight: 1.3,
  padding: "1px 0",
  paddingLeft: depth * 12,
  userSelect: "none",
});
const chevronSpacer: React.CSSProperties = {
  display: "inline-block",
  width: CHEVRON_GUTTER,
  flexShrink: 0,
};
const chevronButton: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: CHEVRON_GUTTER,
  height: "1em",
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-muted)",
  cursor: "pointer",
  flexShrink: 0,
  fontSize: "inherit",
  lineHeight: 1,
};
const iconWrapper: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  // Force the icon to render at the same height as the text so a
  // larger inline emoji / svg can't push the row taller than the
  // others (which would create uneven row heights).
  width: "1em",
  height: "1em",
  fontSize: "1em",
  lineHeight: 1,
  flexShrink: 0,
  color: "var(--text-muted)",
};
const labelButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  margin: 0,
  cursor: "pointer",
  color: "var(--text-primary)",
  fontSize: "inherit",
  fontFamily: "inherit",
  textAlign: "left",
};
const labelText: React.CSSProperties = {
  color: "var(--text-primary)",
};
const detailText: React.CSSProperties = {
  color: "var(--text-muted)",
  marginLeft: 4,
};
const countText: React.CSSProperties = {
  color: "var(--text-muted)",
  fontSize: 11,
  marginLeft: 4,
};
const badgeRow: React.CSSProperties = {
  display: "inline-flex",
  gap: 2,
  flexShrink: 0,
};
const badgeStyle = (cfg: { fg: string; bg: string }): React.CSSProperties => ({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: "1.1em",
  height: "1.1em",
  borderRadius: 3,
  fontSize: "0.75em",
  fontWeight: 700,
  color: cfg.fg,
  background: cfg.bg,
  lineHeight: 1,
});


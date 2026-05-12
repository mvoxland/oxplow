import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { AgentStatus } from "../../api.js";
import { kindForTabId, PageKindIcon } from "../../pageKinds.js";
import { AgentStatusDot } from "../AgentStatusDot.js";
import { Kebab } from "../Kebab.js";
import type { MenuItem } from "../../menu.js";
import { ErrorBoundary } from "../ErrorBoundary.js";

export interface CenterTab {
  id: string;
  label: string;
  closable: boolean;
  render: () => ReactNode;
  agentStatus?: AgentStatus;
  /** Per-tab kebab menu. When present, a `⋯` button appears on the
   *  tab chip; clicking it opens a popover with these entries.
   *  (The legacy right-click affordance was retired in phase 5 of the
   *  IA redesign — visible kebab buttons are the new primary path.)
   */
  contextMenu?: MenuItem[];
  /** Tabs that share a `reorderGroup` can be drag-reordered relative to
   *  each other. Tabs without a group are pinned (e.g. the agent tab). */
  reorderGroup?: string;
  /** When true, this tab does NOT appear in the strip but its body
   *  still mounts (kept hidden via display:none). Used by the host
   *  to keep back/forward stack entries alive so navigation between
   *  them preserves React component state without remounting. */
  hidden?: boolean;
}

interface CenterTabsProps {
  tabs: CenterTab[];
  activeId: string;
  onActivate(id: string): void;
  onClose?(id: string): void;
  /** Rendered above the active tab's content. */
  header?: ReactNode;
  /** Called when the user drag-reorders tabs. Receives the new full id list. */
  onReorder?(orderedIds: string[]): void;
}

const TAB_DRAG_MIME = "application/x-oxplow-center-tab";

export function CenterTabs({ tabs, activeId, onActivate, onClose, header, onReorder }: CenterTabsProps) {
  const active = tabs.find((t) => t.id === activeId) ?? tabs.find((t) => !t.hidden) ?? tabs[0] ?? null;
  const stripTabs = tabs.filter((t) => !t.hidden);
  const [hoverId, setHoverId] = useState<string | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const draggingTab = draggingId ? tabs.find((t) => t.id === draggingId) ?? null : null;
  const stripScrollRef = useRef<HTMLDivElement>(null);
  // The outer flex container that wraps the strip + overflow button.
  // We measure against its width (stable) rather than the strip's
  // own width — the strip shrinks once we hide a tab, which would
  // otherwise cascade into "every tab is too wide" and hide them all.
  const outerBarRef = useRef<HTMLDivElement>(null);
  const overflowButtonRef = useRef<HTMLButtonElement>(null);
  const [overflowOpen, setOverflowOpen] = useState(false);
  // Tabs whose natural right edge would extend past the strip's
  // visible area. We render them `display: none` in the strip so the
  // user never sees a half-clipped chip — they're still listed in the
  // overflow panel. Tab widths are cached so a freshly-hidden tab
  // doesn't lose its width when re-measurement runs (its offsetWidth
  // becomes 0 once display:none).
  const widthCacheRef = useRef<Map<string, number>>(new Map());
  const [hiddenInStripIds, setHiddenInStripIds] = useState<Set<string>>(new Set());
  const hasOverflow = hiddenInStripIds.size > 0;

  useLayoutEffect(() => {
    const outer = outerBarRef.current;
    const strip = stripScrollRef.current;
    if (!outer || !strip) return;
    const OVERFLOW_BUTTON_RESERVE = 36; // matches button minWidth below
    const measure = () => {
      // Update width cache from currently-visible tab chips.
      const children = Array.from(strip.children) as HTMLElement[];
      for (const child of children) {
        const id = child.dataset.tabId;
        if (!id) continue;
        const w = child.offsetWidth;
        if (w > 0) widthCacheRef.current.set(id, w);
      }
      // Drop cache entries for tabs that no longer exist.
      const liveIds = new Set(stripTabs.map((t) => t.id));
      for (const id of widthCacheRef.current.keys()) {
        if (!liveIds.has(id)) widthCacheRef.current.delete(id);
      }
      // Two-pass fit: first without reserving the overflow-button
      // slot. If everything fits, no button needed. Otherwise retry
      // with the reserve so the button has room.
      const outerWidth = outer.clientWidth;
      const computeHidden = (reserve: number): Set<string> => {
        const avail = outerWidth - reserve;
        const hidden = new Set<string>();
        let used = 0;
        for (const tab of stripTabs) {
          const w = widthCacheRef.current.get(tab.id);
          if (w == null) continue; // not yet measured — assume fits
          if (used + w > avail) {
            hidden.add(tab.id);
          } else {
            used += w;
          }
        }
        return hidden;
      };
      let next = computeHidden(0);
      if (next.size > 0) next = computeHidden(OVERFLOW_BUTTON_RESERVE);
      setHiddenInStripIds((prev) => {
        if (prev.size === next.size && [...prev].every((id) => next.has(id))) return prev;
        return next;
      });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(outer);
    for (const child of Array.from(strip.children)) ro.observe(child);
    return () => ro.disconnect();
  }, [stripTabs]);

  useEffect(() => {
    if (!hasOverflow && overflowOpen) setOverflowOpen(false);
  }, [hasOverflow, overflowOpen]);
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div ref={outerBarRef} style={{ display: "flex", borderBottom: "1px solid var(--border-strong)", background: "var(--surface-tab-inactive)", minHeight: 36, position: "relative" }}>
        <div ref={stripScrollRef} style={{ display: "flex", flex: "0 0 auto", maxWidth: "100%", minWidth: 0, overflow: "hidden" }}>
        {stripTabs.map((tab) => {
          const isActive = tab.id === active?.id;
          const isHover = !isActive && hoverId === tab.id;
          const canDrag = !!onReorder && !!tab.reorderGroup;
          const isDropTarget =
            !!draggingTab &&
            !!tab.reorderGroup &&
            tab.reorderGroup === draggingTab.reorderGroup &&
            overId === tab.id &&
            draggingId !== tab.id;
          const hiddenInStrip = hiddenInStripIds.has(tab.id);
          return (
            <div
              key={tab.id}
              data-testid={`center-tab-${tab.id}`}
              data-tab-id={tab.id}
              draggable={canDrag}
              onClick={() => onActivate(tab.id)}
              onMouseEnter={() => setHoverId(tab.id)}
              onMouseLeave={() => setHoverId((prev) => (prev === tab.id ? null : prev))}
              onDragStart={canDrag ? (event) => {
                event.dataTransfer.setData(TAB_DRAG_MIME, tab.id);
                event.dataTransfer.effectAllowed = "move";
                setDraggingId(tab.id);
              } : undefined}
              onDragEnd={canDrag ? () => {
                setDraggingId(null);
                setOverId(null);
              } : undefined}
              onDragOver={onReorder ? (event) => {
                if (!draggingTab) return;
                if (!tab.reorderGroup || tab.reorderGroup !== draggingTab.reorderGroup) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                if (overId !== tab.id) setOverId(tab.id);
              } : undefined}
              onDragLeave={onReorder ? () => {
                if (overId === tab.id) setOverId(null);
              } : undefined}
              onDrop={onReorder ? (event) => {
                if (!draggingTab) return;
                if (!tab.reorderGroup || tab.reorderGroup !== draggingTab.reorderGroup) return;
                event.preventDefault();
                const sourceId = draggingTab.id;
                const targetId = tab.id;
                setDraggingId(null);
                setOverId(null);
                if (sourceId === targetId) return;
                const ids = tabs.map((t) => t.id);
                const fromIdx = ids.indexOf(sourceId);
                const toIdx = ids.indexOf(targetId);
                if (fromIdx < 0 || toIdx < 0) return;
                const next = ids.slice();
                const [moved] = next.splice(fromIdx, 1);
                next.splice(toIdx, 0, moved);
                onReorder(next);
              } : undefined}
              style={{
                padding: "10px 14px",
                background: isActive
                  ? "var(--surface-tab-active)"
                  : isHover
                    ? "var(--surface-card)"
                    : "transparent",
                color: isActive ? "var(--accent)" : "var(--text-secondary)",
                borderRight: "1px solid var(--border-strong)",
                borderTop: isActive ? "1px solid var(--border-strong)" : "1px solid transparent",
                borderLeft: isActive ? "1px solid var(--border-strong)" : "1px solid transparent",
                borderBottom: isActive
                  ? "3px solid var(--accent)"
                  : isDropTarget
                    ? "3px dashed var(--accent)"
                    : "3px solid transparent",
                outline: isDropTarget ? "1px dashed var(--accent)" : "none",
                outlineOffset: isDropTarget ? -2 : 0,
                opacity: draggingId === tab.id ? 0.5 : 1,
                cursor: canDrag ? "grab" : "pointer",
                fontSize: 13,
                fontWeight: isActive ? 600 : 400,
                display: hiddenInStrip ? "none" : "inline-flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} /> : null}
              <PageKindIcon
                kind={kindForTabId(tab.id)}
                size={13}
                style={{ color: "var(--text-secondary)", flexShrink: 0 }}
              />
              <span
                title={tab.label}
                style={{
                  maxWidth: 180,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  display: "inline-block",
                  verticalAlign: "middle",
                }}
              >
                {tab.label}
              </span>
              {tab.contextMenu && tab.contextMenu.length > 0 ? (
                <span onClick={(e) => e.stopPropagation()}>
                  <Kebab items={tab.contextMenu} testId={`center-tab-kebab-${tab.id}`} size={14} />
                </span>
              ) : null}
              {tab.closable && onClose ? (
                <button type="button"
                  data-testid={`center-tab-close-${tab.id}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(tab.id);
                  }}
                  title={`Close ${tab.label}`}
                  style={{
                    border: "none",
                    background: "transparent",
                    color: "var(--muted)",
                    cursor: "pointer",
                    padding: "0 2px",
                    fontSize: 14,
                    lineHeight: 1,
                  }}
                >
                  ×
                </button>
              ) : null}
            </div>
          );
        })}
        </div>
        {hasOverflow ? (
          <button
            ref={overflowButtonRef}
            type="button"
            data-testid="center-tabs-overflow-button"
            aria-label="Show all tabs"
            title="Show all tabs"
            onClick={(e) => {
              e.stopPropagation();
              setOverflowOpen((v) => !v);
            }}
            style={{
              flex: "1 1 auto",
              minWidth: 36,
              alignSelf: "stretch",
              padding: "0 10px",
              border: "none",
              borderLeft: "1px solid var(--border-strong)",
              background: "var(--surface-tab-inactive)",
              color: "var(--text-secondary)",
              cursor: "pointer",
              fontSize: 18,
              lineHeight: 1,
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "flex-start",
            }}
          >
            ▾
          </button>
        ) : null}
        {overflowOpen ? (
          <OverflowPanel
            tabs={stripTabs}
            activeId={active?.id ?? null}
            anchorRef={overflowButtonRef}
            onActivate={(id) => {
              setOverflowOpen(false);
              // If the user picked a tab that was hidden from the
              // strip (cut off), surface it by reordering it
              // just past the leading pinned tabs (the agent
              // tab and anything else without a reorderGroup).
              // The clicked tab lands at the front of the
              // reorderable region, which is "right after the
              // agent tab" in practice.
              if (onReorder && hiddenInStripIds.has(id)) {
                const ids = tabs.map((t) => t.id);
                const fromIdx = ids.indexOf(id);
                // First index whose tab IS reorderable (has a
                // reorderGroup). If no such tab exists, fall back to
                // 1 so we still land just after the agent tab.
                let insertIdx = tabs.findIndex((t) => !!t.reorderGroup);
                if (insertIdx < 0) insertIdx = Math.min(1, tabs.length);
                if (fromIdx >= 0 && fromIdx !== insertIdx) {
                  const next = ids.slice();
                  const [moved] = next.splice(fromIdx, 1);
                  // After removing the source, the insert index
                  // may have shifted by one if source was before it.
                  const adjusted = fromIdx < insertIdx ? insertIdx - 1 : insertIdx;
                  next.splice(adjusted, 0, moved);
                  onReorder(next);
                }
              }
              onActivate(id);
            }}
            onClose={onClose}
            onDismiss={() => setOverflowOpen(false)}
          />
        ) : null}
      </div>
      {header}
      <div style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "hidden", display: "flex", flexDirection: "column", position: "relative" }}>
        {/* Render every tab body as a sibling, only the active one
         *  visible. Hidden tabs (back/forward stack entries the host
         *  pushes for each slot) stay mounted so navigating back to
         *  them preserves their React state — scroll position,
         *  expanded trees, draft text, etc. */}
        {tabs.map((tab) => {
          const isActive = tab.id === active?.id;
          return (
            <div
              key={tab.id}
              style={{
                display: isActive ? "flex" : "none",
                flex: 1,
                minHeight: 0,
                minWidth: 0,
                flexDirection: "column",
              }}
              data-testid={`center-tab-body-${tab.id}`}
              aria-hidden={!isActive}
            >
              <ErrorBoundary label={tab.label}>
                {tab.render()}
              </ErrorBoundary>
            </div>
          );
        })}
      </div>
    </div>
  );
}

interface OverflowPanelProps {
  tabs: CenterTab[];
  activeId: string | null;
  anchorRef: React.RefObject<HTMLButtonElement | null>;
  onActivate(id: string): void;
  onClose?(id: string): void;
  onDismiss(): void;
}

function OverflowPanel({ tabs, activeId, anchorRef, onActivate, onClose, onDismiss }: OverflowPanelProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null);

  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    setPos({ top: rect.bottom + 2, right: Math.max(8, window.innerWidth - rect.right) });
  }, [anchorRef]);

  useEffect(() => {
    function onPointerDown(e: MouseEvent) {
      if (rootRef.current?.contains(e.target as Node)) return;
      if (anchorRef.current?.contains(e.target as Node)) return;
      onDismiss();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onDismiss();
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [anchorRef, onDismiss]);

  if (!pos) return null;
  return (
    <div
      ref={rootRef}
      data-testid="center-tabs-overflow-panel"
      style={{
        position: "fixed",
        top: pos.top,
        right: pos.right,
        zIndex: 1000,
        background: "var(--surface-card)",
        border: "1px solid var(--border-strong)",
        boxShadow: "0 6px 20px rgba(0,0,0,0.25)",
        minWidth: 240,
        maxWidth: 360,
        maxHeight: "60vh",
        overflowY: "auto",
        padding: "4px 0",
      }}
    >
      {tabs.map((tab) => {
        const isActive = tab.id === activeId;
        return (
          <div
            key={tab.id}
            data-testid={`center-tabs-overflow-item-${tab.id}`}
            onClick={() => onActivate(tab.id)}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 6,
              padding: "6px 10px",
              cursor: "pointer",
              background: isActive ? "var(--surface-tab-active)" : "transparent",
              color: isActive ? "var(--accent)" : "var(--text-primary)",
              fontWeight: isActive ? 600 : 400,
              fontSize: 13,
            }}
            onMouseEnter={(e) => {
              if (!isActive) (e.currentTarget as HTMLDivElement).style.background = "var(--surface-hover)";
            }}
            onMouseLeave={(e) => {
              if (!isActive) (e.currentTarget as HTMLDivElement).style.background = "transparent";
            }}
          >
            {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} /> : null}
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={tab.label}>
              {tab.label}
            </span>
            {tab.closable && onClose ? (
              <button
                type="button"
                data-testid={`center-tabs-overflow-close-${tab.id}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onClose(tab.id);
                }}
                title={`Close ${tab.label}`}
                style={{
                  border: "none",
                  background: "transparent",
                  color: "var(--muted)",
                  cursor: "pointer",
                  padding: "0 2px",
                  fontSize: 14,
                  lineHeight: 1,
                }}
              >
                ×
              </button>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

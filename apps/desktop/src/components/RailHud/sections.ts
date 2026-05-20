import type { ThreadWorkState, Task } from "../../api.js";
import type { TabRef } from "../../tabs/tabState.js";
import {
  archivedRef,
  backlogRef,
  changeAnalysisRef,
  dashboardRef,
  doneWorkRef,
  gitDashboardRef,
  indexRef,
  tasksRef,
  uncommittedChangesRef,
} from "../../tabs/pageRefs.js";

export interface PageDirectoryEntry {
  id: string;
  label: string;
  ref: TabRef;
  badge?: number;
}

/**
 * Ids in `computePagesDirectory` that the rail's "Pages" section
 * actually renders. The full directory remains the discovery surface
 * for QuickOpen; the rail keeps a curated subset so it doesn't sprawl.
 */
export const RAIL_PAGE_IDS: ReadonlySet<string> = new Set([
  "tasks",
  "wiki-index",
  "files",
  "git-dashboard",
]);

/**
 * Static directory of top-level pages. Used by QuickOpen as the
 * "launcher" surface (every page is discoverable there) and filtered
 * down to `RAIL_PAGE_IDS` for the rail's compact "Pages" section.
 * Pure helper so it can be unit-tested without mounting the React
 * component. `backlogReadyCount` controls the badge on "Backlog".
 */
export function computePagesDirectory(opts: { backlogReadyCount: number }): PageDirectoryEntry[] {
  return [
    // Labels are emoji-free — `PageKindIcon` resolves the leading
    // glyph from the entry's ref kind at render time.
    { id: "tasks", label: "Tasks", ref: tasksRef() },
    { id: "done-work", label: "Done Work", ref: doneWorkRef() },
    {
      id: "backlog",
      label: "Backlog",
      ref: backlogRef(),
      badge: opts.backlogReadyCount > 0 ? opts.backlogReadyCount : undefined,
    },
    { id: "archived", label: "Archived", ref: archivedRef() },
    { id: "wiki-index", label: "Wiki", ref: indexRef("wiki-index") },
    { id: "files", label: "Files", ref: indexRef("files") },
    { id: "terminal", label: "Terminal", ref: indexRef("terminal") },
    { id: "code-quality", label: "Code Quality", ref: indexRef("code-quality") },
    { id: "local-history", label: "Local History", ref: indexRef("local-history") },
    { id: "git-dashboard", label: "Git", ref: gitDashboardRef() },
    { id: "uncommitted-changes", label: "Uncommitted", ref: uncommittedChangesRef() },
    { id: "change-analysis", label: "Change Analysis", ref: changeAnalysisRef("working") },
    { id: "hook-events", label: "Hook Events", ref: indexRef("hook-events") },
    { id: "subsystem-docs", label: "Subsystem Docs", ref: indexRef("subsystem-docs") },
    { id: "settings", label: "Settings", ref: indexRef("settings") },
    { id: "dashboard-planning", label: "Planning", ref: dashboardRef("planning") },
    { id: "dashboard-review", label: "Review", ref: dashboardRef("review") },
    { id: "dashboard-quality", label: "Quality", ref: dashboardRef("quality") },
    { id: "dashboard-visits", label: "Visits", ref: dashboardRef("visits") },
  ];
}

/**
 * Pick the lowest-sort_index `in_progress` non-epic item from a thread's
 * work state. The "Active item" rail section anchors on this.
 *
 * The store's `inProgress` bucket holds `in_progress` items. The rail's
 * "Active item" means *what the agent is doing right now*.
 */
export function computeActiveItem(state: ThreadWorkState | null): Task | null {
  if (!state) return null;
  const epicIds = new Set(state.epics.map((e) => e.id));
  const candidates = state.inProgress.filter(
    (item) => item.status === "in_progress" && !epicIds.has(item.id),
  );
  if (candidates.length === 0) return null;
  return candidates.reduce((best, current) =>
    current.sort_index < best.sort_index ? current : best,
  );
}

/**
 * If the active in-progress item is a child of an epic, return the epic
 * and its non-archived children (sorted by sort_index ascending). When
 * the active item is standalone, returns null.
 */
export function computeActiveEpicContext(
  state: ThreadWorkState | null,
  active: Task | null,
): { epic: Task; children: Task[] } | null {
  if (!state || !active || !active.parent_id) return null;
  // Only treat the parent as an "epic" if it is in state.epics — i.e. it
  // has children (the runtime classifies any task with children as an
  // epic). A plain task whose id happens to match active.parent_id is
  // not an epic anchor.
  const epic = state.epics.find((i) => i.id === active.parent_id);
  if (!epic) return null;
  const pool = state.items.length > 0 ? state.items : [...state.epics, ...state.inProgress, ...state.waiting, ...state.done];
  const children = pool
    .filter((i) => i.parent_id === epic.id && i.status !== "archived")
    .sort((a, b) => a.sort_index - b.sort_index);
  return { epic, children };
}

/**
 * Return the next-up `ready` items, sorted by sort_index ascending,
 * truncated to `limit`. The "Ready" rail section uses this.
 */
export function computeUpNext(state: ThreadWorkState | null, limit = 5): Task[] {
  if (!state) return [];
  const ready = state.items.filter((item) => item.status === "ready");
  ready.sort((a, b) => a.sort_index - b.sort_index);
  return ready.slice(0, limit);
}

export interface RecentFileEntry {
  path: string;
  touchedAt: number;
}

/** Sort recent files newest-first; truncate to `limit`. */
export function sortRecentFiles(entries: RecentFileEntry[], limit = 8): RecentFileEntry[] {
  return [...entries].sort((a, b) => b.touchedAt - a.touchedAt).slice(0, limit);
}

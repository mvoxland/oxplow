import { useEffect, useMemo, useState, type ComponentProps } from "react";
import type { ThreadWorkState } from "../../api.js";
import { PlanPane } from "./PlanPane.js";
import {
  TasksFilterBar,
  applyTasksFilters,
  loadTasksFilters,
  saveTasksFilters,
  type TasksFilters,
} from "./TasksFilterBar.js";

type PlanPaneProps = ComponentProps<typeof PlanPane>;

/**
 * Composed list shell for the Tasks page: priority filter bar above
 * + PlanPane below. Holds the filter state, persists it to local-
 * storage, and preprocesses `threadWork.items` by priority before
 * handing it to PlanPane. The section split (Ready / Blocked / Done
 * preview) replaces what the search/status/hide-auto/show-closed
 * controls used to provide.
 */
export function TasksList(props: Omit<PlanPaneProps, "hideAuto" | "onlyStatuses" | "excludeStatuses">) {
  const [filters, setFiltersState] = useState<TasksFilters>(() => loadTasksFilters());
  useEffect(() => { saveTasksFilters(filters); }, [filters]);
  const setFilters = (next: TasksFilters) => setFiltersState(next);

  const filteredThreadWork = useMemo<ThreadWorkState | null>(() => {
    if (!props.threadWork) return null;
    if (filters.priorities.size === 0) return props.threadWork;
    const filteredItems = applyTasksFilters(props.threadWork.items, filters);
    const allowedIds = new Set(filteredItems.map((i) => i.id));
    return {
      ...props.threadWork,
      items: filteredItems,
      waiting: props.threadWork.waiting.filter((i) => allowedIds.has(i.id)),
      inProgress: props.threadWork.inProgress.filter((i) => allowedIds.has(i.id)),
      done: props.threadWork.done.filter((i) => allowedIds.has(i.id)),
      epics: props.threadWork.epics.filter((i) => allowedIds.has(i.id)),
    };
  }, [props.threadWork, filters]);

  // visibleSections in props takes precedence — Tasks page passes
  // ["ready", "blocked", "done"] and we don't override.
  return (
    <div
      data-tasks-roomy
      style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
    >
      {/* Tasks-page-only spacing overrides. Promotes the section
          headers from sidebar-density (11px uppercase) to wiki-page
          headings, and gives rows real breathing room. Scoped to
          [data-tasks-roomy] so the same TaskGroupList component still
          renders compactly on Plan Work / Done Work / Archived. */}
      <style>{`
        [data-tasks-roomy] [data-testid^="plan-section-header-"] {
          padding: 24px 24px 12px !important;
          font-size: 18px !important;
          font-weight: 600 !important;
          text-transform: none !important;
          letter-spacing: 0 !important;
          color: var(--text-primary) !important;
          background: transparent !important;
          border-top: none !important;
          border-bottom: 1px solid var(--border-subtle) !important;
          position: static !important;
        }
      `}</style>
      <TasksFilterBar filters={filters} onChange={setFilters} />
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: "auto" }}>
        <PlanPane
          {...props}
          threadWork={filteredThreadWork}
          excludeStatuses={["canceled", "archived"]}
        />
      </div>
    </div>
  );
}

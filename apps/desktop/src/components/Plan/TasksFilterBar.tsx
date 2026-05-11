import type { CSSProperties } from "react";
import type { TaskPriority, TaskStatus } from "../../api.js";

export interface TasksFilters {
  priorities: ReadonlySet<TaskPriority>;
}

const PRIORITIES: TaskPriority[] = ["urgent", "high", "medium", "low"];

const barStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  flexWrap: "wrap",
  padding: "6px 10px",
  borderBottom: "1px solid var(--border)",
  background: "var(--bg-2)",
  fontSize: 12,
};

const chipStyle: CSSProperties = {
  border: "1px solid var(--border)",
  borderRadius: 12,
  padding: "1px 8px",
  background: "var(--bg-1)",
  cursor: "pointer",
  userSelect: "none",
};

const chipOnStyle: CSSProperties = {
  ...chipStyle,
  background: "var(--accent-soft-bg, var(--accent))",
  color: "var(--accent-on, #fff)",
  borderColor: "var(--accent)",
};

/**
 * Filter bar shown above the Tasks page list. Renders priority chips
 * only; the section split (Ready / Blocked / Done preview) and drag-
 * reorder cover what the search/status/hide-auto/show-closed knobs
 * used to do, so they were removed.
 *
 * Toggling a chip on filters to items matching that priority. With no
 * chips on, no client-side filtering is applied.
 */
export function TasksFilterBar({
  filters,
  onChange,
}: {
  filters: TasksFilters;
  onChange(next: TasksFilters): void;
}) {
  const togglePriority = (p: TaskPriority) => {
    const next = new Set(filters.priorities);
    if (next.has(p)) next.delete(p); else next.add(p);
    onChange({ ...filters, priorities: next });
  };
  return (
    <div style={barStyle} data-testid="tasks-filter-bar">
      <span style={{ color: "var(--muted)" }}>Priority:</span>
      {PRIORITIES.map((p) => (
        <button
          key={p}
          type="button"
          onClick={() => togglePriority(p)}
          style={filters.priorities.has(p) ? chipOnStyle : chipStyle}
          data-testid={`tasks-filter-priority-${p}`}
        >
          {p}
        </button>
      ))}
    </div>
  );
}

export const DEFAULT_TASKS_FILTERS: TasksFilters = {
  priorities: new Set(),
};

const STORAGE_KEY = "tasks-filters";

export function loadTasksFilters(): TasksFilters {
  if (typeof window === "undefined") return DEFAULT_TASKS_FILTERS;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_TASKS_FILTERS;
    const parsed = JSON.parse(raw) as { priorities?: string[] };
    return {
      priorities: new Set((parsed.priorities ?? []) as TaskPriority[]),
    };
  } catch {
    return DEFAULT_TASKS_FILTERS;
  }
}

export function saveTasksFilters(filters: TasksFilters): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({
      priorities: [...filters.priorities],
    }));
  } catch { /* ignore quota */ }
}

export function applyTasksFilters<T extends { priority: TaskPriority; status: TaskStatus }>(
  items: T[],
  filters: TasksFilters,
): T[] {
  if (filters.priorities.size === 0) return items;
  return items.filter((item) => filters.priorities.has(item.priority));
}

import type { ThreadWorkState } from "../../api.js";

/**
 * Tasks page view scope. The page can show:
 *  - currentThread: the thread the rest of the app is focused on
 *  - thread: a specific (stream, thread) the user picked
 *  - stream: every thread in the picked stream, merged together
 *  - all: every thread in every stream, merged together
 *
 * Cross-thread/stream views are read-only — mutations are scoped to a
 * single (stream, thread) by the existing handlers, so the picker
 * disables them when the view spans multiple threads.
 */
export type TasksScope =
  | { kind: "currentThread" }
  | { kind: "thread"; streamId: string; threadId: string }
  | { kind: "stream"; streamId: string }
  | { kind: "all" };

export const STORAGE_KEY = "tasks-scope";

export function loadScope(): TasksScope {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { kind: "currentThread" };
    const parsed = JSON.parse(raw) as TasksScope;
    if (parsed && typeof parsed === "object" && "kind" in parsed) return parsed;
  } catch {}
  return { kind: "currentThread" };
}

export function saveScope(scope: TasksScope): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(scope));
  } catch {}
}

/**
 * Merge multiple ThreadWorkStates into a single synthetic state so the
 * existing TasksList/PlanPane render path can show items across threads.
 * The result's `threadId` is empty since the rows came from many threads;
 * mutation handlers already need a real threadId so the page treats this
 * merged state as read-only.
 */
export function mergeThreadWork(states: ThreadWorkState[]): ThreadWorkState {
  return {
    threadId: "",
    waiting: states.flatMap((s) => s.waiting ?? []),
    inProgress: states.flatMap((s) => s.inProgress ?? []),
    done: states.flatMap((s) => s.done ?? []),
    epics: states.flatMap((s) => s.epics ?? []),
    items: states.flatMap((s) => s.items ?? []),
    followups: states.flatMap((s) => s.followups ?? []),
  } as ThreadWorkState;
}

export function isReadOnlyScope(scope: TasksScope): boolean {
  return scope.kind === "stream" || scope.kind === "all";
}

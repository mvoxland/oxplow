import { useEffect, useRef } from "react";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";

/**
 * localStorage-backed per-page snapshot store.
 *
 * Pages opt in via `usePageSnapshot<T>({ serialize, restore, deps })`
 * — on mount, the hook reads any saved blob keyed by the page's
 * `pageKey` (threadId + tabId) and calls `restore(blob)`. On each
 * `deps` change, it calls `serialize()` and persists the result.
 *
 * The snapshot layer is the cross-restart fidelity tier. In-session
 * back/forward keeps using the display:none mounted-stack approach
 * (perfect fidelity, free); snapshots only matter when the React tree
 * is rebuilt after an app restart, when the back-stack is evicted, or
 * when the user navigates between two un-mounted siblings.
 *
 * The hook is a no-op outside a `PageNavigationContext` (so it's
 * safe to use in components that also render inside modal flows).
 */
const SNAPSHOT_STORAGE_KEY = "oxplow.page-snapshots.v1";

interface SnapshotStoreShape {
  [pageKey: string]: unknown;
}

function readSnapshotStore(): SnapshotStoreShape {
  try {
    const raw = window.localStorage.getItem(SNAPSHOT_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as SnapshotStoreShape) : {};
  } catch {
    return {};
  }
}

function writeSnapshotStore(store: SnapshotStoreShape): void {
  try {
    window.localStorage.setItem(SNAPSHOT_STORAGE_KEY, JSON.stringify(store));
  } catch {
    /* ignore quota / serialization errors */
  }
}

/** Read a single snapshot by key. Returns null if absent or unparsable. */
export function readPageSnapshot<T>(pageKey: string): T | null {
  const store = readSnapshotStore();
  const value = store[pageKey];
  return (value ?? null) as T | null;
}

/** Write a single snapshot by key. */
export function writePageSnapshot<T>(pageKey: string, value: T): void {
  const store = readSnapshotStore();
  store[pageKey] = value;
  writeSnapshotStore(store);
}

/** Drop a snapshot by key. Called from closePageTab so closed pages
 *  don't leak snapshot blobs forever. */
export function clearPageSnapshot(pageKey: string): void {
  const store = readSnapshotStore();
  if (!(pageKey in store)) return;
  delete store[pageKey];
  writeSnapshotStore(store);
}

export interface UsePageSnapshotOptions<T> {
  /** Compute the current snapshot. Called on each `deps` change. */
  serialize(): T;
  /** Apply a previously-saved snapshot. Called once on mount when
   *  a snapshot exists for the page's pageKey. */
  restore(snapshot: T): void;
  /** Re-serialize whenever any of these change. Same shape as
   *  `useEffect`'s dep array. */
  deps: unknown[];
}

export function usePageSnapshot<T>({ serialize, restore, deps }: UsePageSnapshotOptions<T>): void {
  const ctx = useOptionalPageNavigation();
  const pageKey = ctx?.pageKey ?? null;
  const restoredKeyRef = useRef<string | null>(null);
  const wroteSinceRestoreRef = useRef(false);

  // Restore once per pageKey. If the host's pageKey changes (rare —
  // e.g. when the same EditorPane instance switches files via the
  // open-files toolbar), we re-restore for the new key.
  useEffect(() => {
    if (!pageKey) return;
    if (restoredKeyRef.current === pageKey) return;
    restoredKeyRef.current = pageKey;
    wroteSinceRestoreRef.current = false;
    const saved = readPageSnapshot<T>(pageKey);
    if (saved !== null) {
      try {
        restore(saved);
      } catch {
        /* page restore is best-effort */
      }
    }
    // Intentionally only depends on pageKey; restore is a stable
    // closure passed by the caller (deps array drives the writes).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageKey]);

  // Persist on dep change. Skip the first run after a restoration
  // so we don't clobber the restored state with the freshly-mounted
  // defaults (which hadn't yet been overwritten by `restore`).
  useEffect(() => {
    if (!pageKey) return;
    if (!wroteSinceRestoreRef.current) {
      wroteSinceRestoreRef.current = true;
      return;
    }
    let value: T;
    try {
      value = serialize();
    } catch {
      return;
    }
    writePageSnapshot(pageKey, value);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageKey, ...deps]);
}

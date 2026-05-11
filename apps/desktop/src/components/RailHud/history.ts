/**
 * Ref kinds that should NOT be recorded as page visits. The agent
 * terminal is always-present, and creation pages have throwaway ids.
 */
export const NON_TRACKED_KINDS: ReadonlySet<string> = new Set([
  "agent",
  "new-stream",
  "new-task",
]);

/** Kinds excluded from the rail History display (still recorded for analytics).
 *  The agent terminal is always-present in the rail, so it would be noise
 *  in History; everything else (including pages pinned in the curated
 *  "Pages" section) is allowed through. */
export const RAIL_HISTORY_EXCLUDE_KINDS: string[] = [
  "agent",
];

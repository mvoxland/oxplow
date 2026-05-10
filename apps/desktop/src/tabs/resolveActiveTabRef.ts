import { agentRef, fileRef } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

/**
 * Resolve the active center-tab id back to a structured TabRef.
 *
 * The desktop's central page-visit recorder watches `effectiveCenterActive`
 * (a string id) and needs the corresponding TabRef to record the kind +
 * payload. Most kinds live in `pageTabs`; the agent terminal is implicit;
 * file tabs live in a separate `fileSessions.openOrder` array keyed by
 * path with the id shape `file:<path>`.
 *
 * Returns `null` when the id can't be resolved (e.g. mid-snap-back, or a
 * stale id from before the active set settled). Callers should treat
 * `null` as "skip recording this transition".
 */
export function resolveActiveTabRef(
  activeId: string,
  pageTabs: TabRef[],
  openFilePaths: string[],
): TabRef | null {
  if (activeId === "agent") return agentRef();
  const fromPage = pageTabs.find((t) => t.id === activeId);
  if (fromPage) return fromPage;
  if (activeId.startsWith("file:")) {
    const path = activeId.slice("file:".length);
    if (openFilePaths.includes(path)) return fileRef(path);
  }
  return null;
}

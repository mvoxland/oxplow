/**
 * Renderer shape for one entry in the Backlinks (and Outbound)
 * panel. Decoupled from the SQLite edge row so the renderer can
 * stay kind-agnostic — the IPC reader-mapper turns each
 * `BacklinkEdge` into the `TabRef` + label+subtitle the row needs.
 */
import type { TabRef } from "./tabState.js";

export interface BacklinkEntry {
  ref: TabRef;
  label: string;
  subtitle?: string;
}

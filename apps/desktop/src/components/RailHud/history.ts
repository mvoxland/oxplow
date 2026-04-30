import type { TabRef } from "../../tabs/tabState.js";

/**
 * Derive a default display label for a TabRef. App-level callers should
 * pass an explicit label when richer context is available (work item
 * title, note title, etc.); this is the fallback for static pages and
 * files.
 */
export function deriveDefaultLabel(ref: TabRef): string {
  switch (ref.kind) {
    case "file": {
      const path = (ref.payload as { path?: string } | null)?.path ?? "";
      return path.split("/").pop() ?? path ?? "File";
    }
    case "tasks": return "Tasks";
    case "done-work": return "Done Work";
    case "backlog": return "Backlog";
    case "archived": return "Archived";
    case "notes-index": return "Notes";
    case "files": return "Files";
    case "code-quality": return "Code Quality";
    case "local-history": return "Local History";
    case "git-history": return "Git History";
    case "git-dashboard": return "Git Dashboard";
    case "git-commit": return "Git Commit";
    case "uncommitted-changes": return "Uncommitted";
    case "hook-events": return "Hook Events";
    case "subsystem-docs": return "Subsystem Docs";
    case "settings": return "Settings";
    case "stream-settings": return "Stream Settings";
    case "thread-settings": return "Thread Settings";
    case "new-stream": return "New Stream";
    case "new-work-item": return "New Work Item";
    case "dashboard": {
      const variant = (ref.payload as { variant?: string } | null)?.variant ?? "";
      return variant ? `Dashboard: ${variant}` : "Dashboard";
    }
    case "work-item": {
      const id = (ref.payload as { itemId?: string } | null)?.itemId ?? ref.id;
      return id;
    }
    case "note": {
      const slug = (ref.payload as { slug?: string } | null)?.slug ?? ref.id;
      return slug;
    }
    default:
      return ref.id;
  }
}

/**
 * Ref kinds that should NOT be recorded as page visits. The agent
 * terminal is always-present, and creation pages have throwaway ids.
 */
export const NON_TRACKED_KINDS: ReadonlySet<string> = new Set([
  "agent",
  "new-stream",
  "new-work-item",
]);

/** Kinds excluded from the rail History display (still recorded for analytics).
 *  Includes the kinds already pinned in the rail's curated "Pages" section
 *  so they don't appear twice. */
export const RAIL_HISTORY_EXCLUDE_KINDS: string[] = [
  "agent",
  "new-stream",
  "new-work-item",
  "diff",
  "git-commit",
  "tasks",
  "notes-index",
  "files",
  "git-dashboard",
];

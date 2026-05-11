/**
 * Page-kind scheme metadata.
 *
 * `kindForTabId` extracts the scheme/kind from a tab id like
 * `file:src/foo.ts` → `"file"`, or `tasks` → `"tasks"` (literal
 * index pages have no prefix).
 *
 * `pageKindIconComponent` / `PageKindIcon` map that kind to the
 * lucide-react icon used everywhere a page-kind label renders —
 * tabs, rail history/finished, backlinks list, markdown links.
 *
 * Scheme list lives next to this file's mapping; if you add a new
 * tab scheme in `tabs/pageRefs.ts`, add it here too.
 */
import {
  Activity,
  AlertCircle,
  AlertTriangle,
  Archive,
  BookOpen,
  Bot,
  CheckCheck,
  CheckSquare,
  Copy,
  ExternalLink,
  FileCode,
  FileText,
  Folder,
  FolderTree,
  Gauge,
  GitBranch,
  GitCommit,
  GitCompare,
  GitMerge,
  History,
  Inbox,
  Layers,
  LayoutDashboard,
  Library,
  type LucideIcon,
  Plus,
  Settings,
} from "lucide-react";
import type { ComponentProps, ReactElement } from "react";

/**
 * Map a page-kind string to its icon component. Returns `null`
 * for unknown kinds so the caller can fall back to text-only.
 *
 * Accepts every value that can appear as a `TabRef.kind` or as the
 * literal id of an index page. Strings rather than a typed enum so
 * we can pass through `BacklinkEdge.target_kind` (loose string) and
 * arbitrary `tab.id` prefixes without an exhaustive switch eating
 * future kinds.
 */
export function pageKindIconComponent(kind: string): LucideIcon | null {
  switch (kind) {
    // Scheme-prefixed kinds (TabRef.kind).
    case "file":
      return FileText;
    case "directory":
      return Folder;
    case "diff":
      return GitCompare;
    case "duplicate-block":
      return Copy;
    case "wiki":
      return BookOpen;
    case "task":
      return CheckSquare;
    case "finding":
      return AlertTriangle;
    case "git-commit":
      return GitCommit;
    case "dashboard":
      return LayoutDashboard;
    case "op-error":
      return AlertCircle;
    case "stream-settings":
    case "thread-settings":
    case "settings":
      return Settings;
    case "external-url":
      return ExternalLink;
    case "uncommitted-changes":
      return GitBranch;

    // Non-tab kinds used as backlink rows.
    case "snapshot":
      return Layers;

    // Display-label aliases passed by `<Page kind="...">` callers
    // — these arrived before the kind set was unified so they
    // diverge from the canonical tab-id prefix. Map them through
    // rather than forcing every page component to update in lock-
    // step with this file.
    case "wiki page":
      return BookOpen;
    case "commit":
      return GitCommit;
    case "new tasks":
      return Plus;
    case "threads":
      return Archive;

    // Literal-id index pages (kind === id).
    case "agent":
      return Bot;
    case "tasks":
      return CheckSquare;
    case "done-work":
      return CheckCheck;
    case "backlog":
      return Inbox;
    case "archived":
    case "closed-threads":
      return Archive;
    case "wiki-index":
      return Library;
    case "files":
      return FolderTree;
    case "code-quality":
      return Gauge;
    case "local-history":
      return History;
    case "git-history":
      return GitMerge;
    case "git-dashboard":
      return GitBranch;
    case "hook-events":
      return Activity;
    case "subsystem-docs":
      return FileCode;
    case "new-stream":
    case "new-task":
      return Plus;

    default:
      return null;
  }
}

export interface PageKindIconProps extends Omit<ComponentProps<LucideIcon>, "ref"> {
  kind: string;
  /**
   * Pixel size for the icon. Defaults to 14 — small enough to sit
   * inline next to text labels at the project's default font size.
   */
  size?: number;
}

/**
 * Render the icon for `kind`. Returns `null` for unknown kinds so
 * call sites can interleave `<PageKindIcon …/>` with a label and
 * unrecognized kinds simply lose the leading glyph rather than
 * crashing or rendering a question-mark placeholder.
 *
 * `aria-hidden` is set by default because the adjacent text label
 * is the accessible name; the icon is decorative.
 */
export function PageKindIcon({
  kind,
  size = 14,
  ...rest
}: PageKindIconProps): ReactElement | null {
  const Icon = pageKindIconComponent(kind);
  if (!Icon) return null;
  return <Icon aria-hidden size={size} {...rest} />;
}

/**
 * Index-page ids that double as their own kind — these tab ids
 * carry no scheme prefix and the whole id is the kind label.
 */
const INDEX_KINDS = new Set<string>([
  "agent",
  "tasks",
  "done-work",
  "backlog",
  "archived",
  "wiki-index",
  "files",
  "code-quality",
  "local-history",
  "git-history",
  "git-dashboard",
  "hook-events",
  "subsystem-docs",
  "settings",
  "new-stream",
  "new-task",
  "closed-threads",
  "uncommitted-changes",
]);

/**
 * Parse the scheme/kind from a tab id. Examples:
 *
 *   `file:src/foo.ts`          → `"file"`
 *   `wiki:url-schemes`         → `"wiki"`
 *   `task:42`                  → `"task"`
 *   `git-commit:abc123:scope`  → `"git-commit"`
 *   `tasks`                    → `"tasks"`
 *   `uncommitted-changes:dir:src` → `"uncommitted-changes"`
 *
 * For prefixed ids the kind is everything before the first `:`,
 * except that two-segment literal kinds (`git-commit`,
 * `uncommitted-changes`, `stream-settings`, `thread-settings`,
 * `op-error`, `duplicate-block`, `external-url`, `done-work`,
 * `wiki-index`, `code-quality`, `local-history`, `git-history`,
 * `git-dashboard`, `hook-events`, `subsystem-docs`, `new-stream`,
 * `new-task`, `closed-threads`) are hyphenated — the colon split
 * still works for those because the hyphen comes before any `:`.
 */
export function kindForTabId(tabId: string): string {
  if (INDEX_KINDS.has(tabId)) return tabId;
  const idx = tabId.indexOf(":");
  if (idx === -1) return tabId;
  return tabId.slice(0, idx);
}

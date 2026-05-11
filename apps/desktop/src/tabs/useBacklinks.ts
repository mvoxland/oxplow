import { useEffect, useState } from "react";
import { listBacklinks, listPageOutbound, type BacklinkEdge } from "../api.js";
import {
  directoryRef,
  fileRef,
  findingRef,
  gitCommitRef,
  wikiPageRef,
  taskRef,
} from "./pageRefs.js";
import type { TabRef } from "./tabState.js";
import type { BacklinkEntry } from "./backlinkTypes.js";

/**
 * Backlinks for a page. One IPC call to the unified `page_ref`
 * graph; the SQLite reader joins source labels (wiki title, work-
 * item title, commit subject) at read time so the renderer doesn't
 * need a second round-trip per row.
 *
 * Replaces the old in-memory `computeBacklinks` indexer + per-kind
 * `appPageBacklinks` providers — every page kind goes through the
 * same code path now, including FilePage (which used to render an
 * empty list because it never wired the indexer).
 */
export function useBacklinks(target: TabRef): BacklinkEntry[] {
  const [entries, setEntries] = useState<BacklinkEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    const targetId = canonicalIdForTarget(target);
    if (!targetId) {
      setEntries([]);
      return;
    }
    void listBacklinks(target.kind, targetId, null)
      .then((edges) => {
        if (cancelled) return;
        const mapped = edges
          .map(edgeToInboundEntry)
          .filter((e): e is BacklinkEntry => e !== null);
        setEntries(dedupeEntriesByTarget(mapped));
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [target.kind, target.id]);

  return entries;
}

/**
 * Outbound: what this page points AT. Sibling to `useBacklinks` and
 * drives the new Outbound dropdown / panel in the Page chrome.
 */
export function usePageOutbound(source: TabRef): BacklinkEntry[] {
  const [entries, setEntries] = useState<BacklinkEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    const sourceId = canonicalIdForTarget(source);
    if (!sourceId) {
      setEntries([]);
      return;
    }
    void listPageOutbound(source.kind, sourceId, null)
      .then((edges) => {
        if (cancelled) return;
        const mapped = edges
          .map(edgeToOutboundEntry)
          .filter((e): e is BacklinkEntry => e !== null);
        setEntries(dedupeEntriesByTarget(mapped));
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [source.kind, source.id]);

  return entries;
}

/**
 * Extract the canonical SQLite id from a TabRef. Most kinds carry
 * their canonical id directly in `payload`; some encode it inside
 * the prefixed `ref.id`. Returns null for kinds the page-ref graph
 * doesn't track (settings, dialogs, …) so the hook short-circuits.
 */
export function canonicalIdForTarget(ref: TabRef): string | null {
  switch (ref.kind) {
    case "wiki": {
      const p = ref.payload as { slug?: string } | null;
      return p?.slug ?? null;
    }
    case "task": {
      // payload.itemId is a number (see `taskRef` in pageRefs.ts);
      // stringify so the Tauri command receives the `String` it
      // declares. Without this the IPC throws on deserialize and
      // the hook silently sets entries to [].
      const p = ref.payload as { itemId?: string | number } | null;
      return p?.itemId != null ? String(p.itemId) : null;
    }
    case "file": {
      const p = ref.payload as { path?: string } | null;
      return p?.path ?? null;
    }
    case "directory": {
      const p = ref.payload as { path?: string } | null;
      return p?.path ?? null;
    }
    case "git-commit": {
      const p = ref.payload as { sha?: string } | null;
      return p?.sha ?? null;
    }
    case "finding": {
      const p = ref.payload as { findingId?: string } | null;
      return p?.findingId ?? null;
    }
    default:
      return null;
  }
}

/** Convert one inbound edge (source -> me) into a renderer entry. */
function edgeToInboundEntry(edge: BacklinkEdge): BacklinkEntry | null {
  const ref = refFor(edge.source_kind, edge.source_id);
  if (!ref) return null;
  const label = edge.source_label ?? edge.source_id;
  const subtitle = humanRefType(edge.ref_type, edge.source_extra);
  return { ref, label, subtitle };
}

/** Convert one outbound edge (me -> target) into a renderer entry. */
function edgeToOutboundEntry(edge: BacklinkEdge): BacklinkEntry | null {
  const ref = refFor(edge.target_kind, edge.target_id);
  if (!ref) return null;
  const label = edge.source_label ?? edge.target_id;
  const subtitle = humanRefType(edge.ref_type, edge.source_extra);
  return { ref, label, subtitle };
}

/**
 * Collapse rows whose `ref` resolves to the same logical page.
 * Two cases that need this today:
 *
 *  1. A wiki page is referenced both as `wiki:<slug>` (via
 *     `summary_wikilink` / `impact`) and as `file:.oxplow/wiki/<slug>.md`
 *     (via `touched_file`). Same underlying entity, three rows.
 *  2. Multiple ref_types pointing at the same target (e.g. an
 *     impact edge AND a summary_wikilink edge to the same slug).
 *
 * Strategy: bucket by canonical key, prefer the wiki ref over the
 * file ref when both exist, prefer a non-fallback label, and merge
 * each contributor's subtitle into a `·`-separated chip line. Order
 * within the input is preserved across the first appearance of each
 * key.
 */
export function dedupeEntriesByTarget(entries: BacklinkEntry[]): BacklinkEntry[] {
  const buckets = new Map<string, BacklinkEntry[]>();
  const order: string[] = [];
  for (const e of entries) {
    const key = canonicalEntryKey(e.ref);
    if (!buckets.has(key)) {
      buckets.set(key, []);
      order.push(key);
    }
    buckets.get(key)!.push(e);
  }
  return order.map((key) => {
    const group = buckets.get(key)!;
    if (group.length === 1) return group[0];
    // Pick the "best" base entry: wiki kind wins over file when the
    // file is the on-disk shadow of the same wiki page.
    const wikiMember = group.find((g) => g.ref.kind === "wiki");
    const base = wikiMember ?? group[0];
    // Combine subtitles, dropping empties and duplicates while
    // preserving first-seen order.
    const seen = new Set<string>();
    const subs: string[] = [];
    for (const g of group) {
      const s = g.subtitle?.trim();
      if (!s || seen.has(s)) continue;
      seen.add(s);
      subs.push(s);
    }
    return {
      ref: base.ref,
      label: base.label,
      subtitle: subs.length > 0 ? subs.join(" · ") : undefined,
    };
  });
}

/**
 * Identity key for dedup. Same key ⇒ same logical page.
 *
 * `file:.oxplow/wiki/<slug>.md` is treated as `wiki:<slug>` so the
 * on-disk shadow of a wiki page doesn't render as a sibling row to
 * the wiki tab itself.
 */
function canonicalEntryKey(ref: TabRef): string {
  if (ref.kind === "file") {
    const path = (ref.payload as { path?: string } | null)?.path;
    if (path) {
      const m = /^\.oxplow\/wiki\/(.+)\.md$/.exec(path);
      if (m) return `wiki:${m[1]}`;
    }
  }
  const id = canonicalIdForTarget(ref) ?? ref.id;
  return `${ref.kind}:${id}`;
}

/**
 * Build a navigable `TabRef` from a (kind, id) pair. Kinds the
 * frontend doesn't render directly (rare today but possible for
 * future page kinds) yield null so the row is dropped rather than
 * rendering as a dead button.
 */
function refFor(kind: string, id: string): TabRef | null {
  switch (kind) {
    case "wiki":
      return wikiPageRef(id);
    case "task": {
      const n = Number(id);
      return Number.isFinite(n) ? taskRef(n) : null;
    }
    case "file":
      return fileRef(id);
    case "directory":
      return directoryRef(id);
    case "git-commit":
      return gitCommitRef(id);
    case "finding":
      return findingRef(id);
    default:
      return null;
  }
}

/**
 * Short label for the relationship type, used as the row's
 * subtitle. For `impact` edges the `source_extra` JSON carries the
 * declared action verb (`{"action":"created"}`) — surface it as
 * `"impact (created)"` so the user can tell what kind of impact.
 */
function humanRefType(refType: string, sourceExtra: string | null): string {
  if (refType.startsWith("task_link:")) {
    const sub = refType.slice("task_link:".length);
    return sub.replace(/_/g, " ");
  }
  switch (refType) {
    case "wiki_file_ref":
      return "wiki link";
    case "wiki_dir_ref":
      return "wiki link";
    case "wikilink":
      return "wiki link";
    case "summary_wikilink":
      return "wiki link";
    case "summary_file_ref":
      return "wiki link";
    case "summary_dir_ref":
      return "wiki link";
    case "task_body_mention":
      return "mention";
    case "summary_task_mention":
      return "mention";
    case "finding_mention":
      return "mention";
    case "summary_finding_mention":
      return "mention";
    case "commit_mention":
      return "mention";
    case "summary_commit_mention":
      return "mention";
    case "touched_file":
      return "touched";
    case "finding_path":
      return "found in";
    case "impact": {
      const action = parseImpactAction(sourceExtra);
      return action ? `impact (${action})` : "impact";
    }
    default:
      return refType;
  }
}

function parseImpactAction(sourceExtra: string | null): string | null {
  if (!sourceExtra) return null;
  try {
    const parsed = JSON.parse(sourceExtra) as { action?: unknown };
    return typeof parsed.action === "string" && parsed.action.length > 0
      ? parsed.action
      : null;
  } catch {
    return null;
  }
}

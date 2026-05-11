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
        setEntries(edges.map(edgeToInboundEntry).filter((e): e is BacklinkEntry => e !== null));
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
        setEntries(
          edges.map(edgeToOutboundEntry).filter((e): e is BacklinkEntry => e !== null),
        );
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
function canonicalIdForTarget(ref: TabRef): string | null {
  switch (ref.kind) {
    case "wiki": {
      const p = ref.payload as { slug?: string } | null;
      return p?.slug ?? null;
    }
    case "task": {
      const p = ref.payload as { itemId?: string } | null;
      return p?.itemId ?? null;
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
  const subtitle = humanRefType(edge.ref_type);
  return { ref, label, subtitle };
}

/** Convert one outbound edge (me -> target) into a renderer entry. */
function edgeToOutboundEntry(edge: BacklinkEdge): BacklinkEntry | null {
  const ref = refFor(edge.target_kind, edge.target_id);
  if (!ref) return null;
  const label = edge.source_label ?? edge.target_id;
  const subtitle = humanRefType(edge.ref_type);
  return { ref, label, subtitle };
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

/** Short label for the relationship type, used as the row's subtitle. */
function humanRefType(refType: string): string {
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
    case "task_body_mention":
      return "mention";
    case "finding_mention":
      return "mention";
    case "commit_mention":
      return "mention";
    case "touched_file":
      return "touched";
    case "finding_path":
      return "found in";
    default:
      return refType;
  }
}

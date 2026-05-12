import { useEffect, useState } from "react";
import { listWikiPages, subscribeWikiPageEvents } from "./api.js";

/**
 * Shared in-memory slug → title map. The wiki body markdown renderer
 * uses this to display `[[some-slug]]` wikilinks with the page's real
 * title instead of the raw slug — readers should see "Local Snapshots"
 * not `local-snapshots`.
 *
 * One load on first subscribe; refreshed when the runtime emits
 * `wikiPagesChanged` (page added, renamed, deleted). Components
 * subscribe via `useWikiTitle(slug)`.
 */

let titles = new Map<string, string>();
let loaded = false;
let inFlight: Promise<void> | null = null;
const listeners = new Set<() => void>();

function notify() {
  for (const fn of listeners) {
    try { fn(); } catch { /* ignore listener errors */ }
  }
}

async function refresh(): Promise<void> {
  if (inFlight) return inFlight;
  inFlight = (async () => {
    try {
      const pages = await listWikiPages("");
      const next = new Map<string, string>();
      for (const p of pages) {
        if (p.slug && p.title) next.set(p.slug, p.title);
      }
      titles = next;
      loaded = true;
      notify();
    } catch {
      // Leave the previous map in place on failure.
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

let unsubscribeEvents: (() => void) | null = null;
function ensureSubscribed() {
  if (unsubscribeEvents) return;
  unsubscribeEvents = subscribeWikiPageEvents(() => {
    void refresh();
  });
}

/**
 * Resolve a wiki slug to its page title. Returns `null` while the
 * cache is still loading or if the slug isn't known (deleted page,
 * stale wikilink, etc.) — callers should fall back to the slug in
 * that case.
 */
export function useWikiTitle(slug: string | null | undefined): string | null {
  const [title, setTitle] = useState<string | null>(() =>
    slug ? titles.get(slug) ?? null : null,
  );

  useEffect(() => {
    if (!slug) {
      setTitle(null);
      return;
    }
    ensureSubscribed();
    const update = () => setTitle(titles.get(slug) ?? null);
    listeners.add(update);
    if (!loaded && !inFlight) {
      void refresh();
    } else {
      update();
    }
    return () => {
      listeners.delete(update);
    };
  }, [slug]);

  return title;
}

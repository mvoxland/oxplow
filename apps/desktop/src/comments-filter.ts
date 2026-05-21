// Comments Dashboard resolved-by-date filtering.
//
// The dashboard defaults to unresolved (open) threads only. A date
// control reveals resolved threads bucketed by how recently they were
// resolved. The bucket thresholds form a *tiered* ladder — one per day
// back to a week, one per week back to a month, then one per month
// beyond — rather than a linear day-by-day list, and the ladder is
// capped at the oldest actual resolved comment so the largest option
// reaches all of them (resolved comments are eventually GC'd, so the
// ladder stays short). Resolution time comes from `comment.resolved_at`
// (the only reliable signal — see the store's set_status).

import type { CommentStatus } from "./tauri-bridge/generated/bindings.js";

const DAY_MS = 86_400_000;

/// Minimal structural shape these helpers read — satisfied by the real
/// `CommentThread` and by test stubs alike.
interface CommentFilterShape {
  comment: { status: CommentStatus; resolved_at: string | null };
}

/// Threads to display: open threads always; resolved threads only when a
/// window is chosen (`resolvedWindowDays` non-null) and they were
/// resolved within it. Order is preserved.
export function visibleThreads<T extends CommentFilterShape>(
  threads: T[],
  resolvedWindowDays: number | null,
  nowMs: number,
): T[] {
  return threads.filter((t) => {
    if (t.comment.status !== "resolved") return true;
    if (resolvedWindowDays == null) return false;
    const at = t.comment.resolved_at;
    if (!at) return false;
    return nowMs - new Date(at).getTime() <= resolvedWindowDays * DAY_MS;
  });
}

/// One selectable resolved-window: a threshold in days plus its label.
/// `days` is what `visibleThreads` filters on; `label` reflects the tier
/// it came from (days / weeks / months).
export interface ResolvedWindowOption {
  days: number;
  label: string;
}

/// The tiered ladder of thresholds up to (and including the first step
/// that covers) `maxDays`: 1..7 days, then 2..4 weeks, then 2,3,… months.
function ladderUpTo(maxDays: number): ResolvedWindowOption[] {
  const out: ResolvedWindowOption[] = [];
  for (let d = 1; d <= 7; d++) {
    out.push({ days: d, label: d === 1 ? "Resolved in the last day" : `Resolved in the last ${d} days` });
    if (d >= maxDays) return out;
  }
  for (let w = 2; w <= 4; w++) {
    out.push({ days: w * 7, label: `Resolved in the last ${w} weeks` });
    if (w * 7 >= maxDays) return out;
  }
  for (let m = 2; ; m++) {
    out.push({ days: m * 30, label: `Resolved in the last ${m} months` });
    if (m * 30 >= maxDays) return out;
  }
}

/// The resolved-window options to offer, derived from the actual oldest
/// resolved comment: empty when nothing is resolved, otherwise the
/// tiered ladder reaching far enough back to cover every resolved one.
export function resolvedWindowOptions<T extends CommentFilterShape>(
  threads: T[],
  nowMs: number,
): ResolvedWindowOption[] {
  let maxAgeDays = 0;
  for (const t of threads) {
    if (t.comment.status !== "resolved") continue;
    const at = t.comment.resolved_at;
    if (!at) continue;
    const ageMs = Math.max(0, nowMs - new Date(at).getTime());
    maxAgeDays = Math.max(maxAgeDays, Math.max(1, Math.ceil(ageMs / DAY_MS)));
  }
  return maxAgeDays === 0 ? [] : ladderUpTo(maxAgeDays);
}

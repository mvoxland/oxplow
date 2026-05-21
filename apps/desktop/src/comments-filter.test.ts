import { describe, expect, test } from "bun:test";
import { resolvedWindowOptions, visibleThreads } from "./comments-filter.js";

const DAY = 86_400_000;
const NOW = Date.parse("2026-05-21T12:00:00Z");

function thread(status: "open" | "resolved", resolvedDaysAgo: number | null) {
  return {
    comment: {
      status,
      resolved_at:
        resolvedDaysAgo == null ? null : new Date(NOW - resolvedDaysAgo * DAY).toISOString(),
    },
  };
}

describe("visibleThreads", () => {
  test("open threads are always shown, regardless of the window", () => {
    const open = thread("open", null);
    expect(visibleThreads([open], null, NOW)).toEqual([open]);
    expect(visibleThreads([open], 7, NOW)).toEqual([open]);
  });

  test("resolved threads are hidden by default (window null)", () => {
    expect(visibleThreads([thread("resolved", 0.1)], null, NOW)).toEqual([]);
  });

  test("resolved threads show only when within the chosen window", () => {
    const recent = thread("resolved", 0.5); // half a day ago
    const old = thread("resolved", 3); // three days ago
    // 1-day window: only the recent one.
    expect(visibleThreads([recent, old], 1, NOW)).toEqual([recent]);
    // 3-day window: both.
    expect(visibleThreads([recent, old], 3, NOW)).toEqual([recent, old]);
  });

  test("resolved with no resolved_at is never bucketed", () => {
    expect(visibleThreads([thread("resolved", null)], 30, NOW)).toEqual([]);
  });
});

describe("resolvedWindowOptions", () => {
  const days = (threads: ReturnType<typeof thread>[]) =>
    resolvedWindowOptions(threads, NOW).map((o) => o.days);

  test("is empty when nothing is resolved", () => {
    expect(resolvedWindowOptions([thread("open", null)], NOW)).toEqual([]);
  });

  test("daily granularity within the first week, capped at the oldest", () => {
    expect(days([thread("resolved", 0)])).toEqual([1]); // resolved moments ago
    expect(days([thread("resolved", 0.2), thread("resolved", 2.2)])).toEqual([1, 2, 3]);
    expect(days([thread("resolved", 7)])).toEqual([1, 2, 3, 4, 5, 6, 7]);
  });

  test("weekly steps between a week and a month", () => {
    // Oldest 10 days → 1..7 then the first weekly step (14) that covers it.
    expect(days([thread("resolved", 10)])).toEqual([1, 2, 3, 4, 5, 6, 7, 14]);
    expect(days([thread("resolved", 25)])).toEqual([1, 2, 3, 4, 5, 6, 7, 14, 21, 28]);
  });

  test("monthly steps beyond a month", () => {
    // Oldest 40 days → daily + weekly + first month step (60).
    expect(days([thread("resolved", 40)])).toEqual([1, 2, 3, 4, 5, 6, 7, 14, 21, 28, 60]);
    expect(days([thread("resolved", 75)])).toEqual([1, 2, 3, 4, 5, 6, 7, 14, 21, 28, 60, 90]);
  });

  test("labels reflect the tier", () => {
    const opts = resolvedWindowOptions([thread("resolved", 75)], NOW);
    const byDays = new Map(opts.map((o) => [o.days, o.label]));
    expect(byDays.get(1)).toBe("Resolved in the last day");
    expect(byDays.get(3)).toBe("Resolved in the last 3 days");
    expect(byDays.get(14)).toBe("Resolved in the last 2 weeks");
    expect(byDays.get(90)).toBe("Resolved in the last 3 months");
  });
});

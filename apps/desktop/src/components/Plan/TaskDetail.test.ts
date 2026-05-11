import { expect, test } from "bun:test";
import type { EffortDetail } from "../../api.js";
import { buildActivityTimeline } from "./TaskDetail.js";

function effort(
  id: string,
  started_at: string,
  ended_at: string | null,
  summary: string | null = null,
): EffortDetail {
  return {
    effort: {
      id,
      work_item_id: "w1",
      started_at,
      ended_at,
      start_snapshot_id: null,
      end_snapshot_id: null,
      summary,
    },
    start_snapshot: null,
    end_snapshot: null,
    changed_paths: [],
    counts: { created: 0, updated: 0, deleted: 0 },
  };
}

test("buildActivityTimeline orders efforts newest-first by ended_at, with active first", () => {
  const efforts = [
    effort("e1", "2026-04-25T09:00:00Z", "2026-04-25T11:00:00Z"),
    effort("e2", "2026-04-25T13:00:00Z", null),
  ];
  const rows = buildActivityTimeline(efforts);
  expect(rows.map((r) => r.id)).toEqual(["e2", "e1"]);
  expect(rows[0].active).toBe(true);
  expect(rows[1].active).toBe(false);
});

test("buildActivityTimeline uses ended_at as primary timestamp for closed efforts", () => {
  const efforts = [
    effort("e1", "2026-04-25T09:00:00Z", "2026-04-25T11:00:00Z"),
  ];
  const rows = buildActivityTimeline(efforts);
  expect(rows[0].timestamp).toBe("2026-04-25T11:00:00Z");
});

test("buildActivityTimeline returns empty list when nothing recorded", () => {
  expect(buildActivityTimeline([])).toEqual([]);
});

import { describe, expect, test } from "bun:test";
import { groupChangesByEffort } from "./snapshot-effort-grouping.js";

const changed = (path: string, status = "modified") => ({ path, status });
const effort = (effortId: string, title: string, files: [string, string][]) => ({
  effortId,
  title,
  files: files.map(([path, change]) => ({ path, change })),
});

describe("groupChangesByEffort", () => {
  test("puts a singly-claimed changed file under its effort", () => {
    const r = groupChangesByEffort(
      [changed("a.ts"), changed("b.ts", "added")],
      [effort("e1", "Task One", [["a.ts", "updated"], ["b.ts", "created"]])],
    );
    expect(r.byEffort).toHaveLength(1);
    expect(r.byEffort[0]).toMatchObject({ effortId: "e1", title: "Task One" });
    expect(r.byEffort[0].files.map((f) => f.entry.path)).toEqual(["a.ts", "b.ts"]);
    expect(r.byEffort[0].files[0]).toMatchObject({ declaredChange: "updated", alsoClaimedBy: [] });
    expect(r.unclaimed).toEqual([]);
    expect(r.idleEffortIds).toEqual([]);
  });

  test("a changed file no effort claims lands in unclaimed", () => {
    const r = groupChangesByEffort(
      [changed("orphan.ts")],
      [effort("e1", "Task One", [["other.ts", "updated"]])],
    );
    expect(r.byEffort).toEqual([]);
    expect(r.unclaimed.map((f) => f.entry.path)).toEqual(["orphan.ts"]);
    // e1 claims nothing that changed here → idle.
    expect(r.idleEffortIds).toEqual(["e1"]);
  });

  test("a file claimed by two efforts appears under each with cross-references", () => {
    const r = groupChangesByEffort(
      [changed("shared.ts")],
      [
        effort("e1", "Task One", [["shared.ts", "updated"]]),
        effort("e2", "Task Two", [["shared.ts", "created"]]),
      ],
    );
    expect(r.byEffort.map((g) => g.effortId)).toEqual(["e1", "e2"]);
    const g1 = r.byEffort.find((g) => g.effortId === "e1")!;
    const g2 = r.byEffort.find((g) => g.effortId === "e2")!;
    expect(g1.files[0]).toMatchObject({ declaredChange: "updated", alsoClaimedBy: ["Task Two"] });
    expect(g2.files[0]).toMatchObject({ declaredChange: "created", alsoClaimedBy: ["Task One"] });
    expect(r.unclaimed).toEqual([]);
    expect(r.idleEffortIds).toEqual([]);
  });

  test("preserves effort order and only lists efforts that claim changed files", () => {
    const r = groupChangesByEffort(
      [changed("x.ts")],
      [
        effort("e1", "First", [["unrelated.ts", "updated"]]),
        effort("e2", "Second", [["x.ts", "updated"]]),
      ],
    );
    expect(r.byEffort.map((g) => g.effortId)).toEqual(["e2"]);
    expect(r.idleEffortIds).toEqual(["e1"]);
  });
});

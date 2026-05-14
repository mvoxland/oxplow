import { describe, expect, test } from "bun:test";
import { formatSnapshotSubject } from "./LocalHistoryDashboardPage.js";

describe("formatSnapshotSubject", () => {
  test("completed efforts win over the isInitial flag", () => {
    expect(
      formatSnapshotSubject([{ title: "fix snapshot" }], [], true),
    ).toBe("completed: fix snapshot");
  });

  test("joins multiple completed effort titles", () => {
    expect(
      formatSnapshotSubject([{ title: "a" }, { title: "b" }], [], false),
    ).toBe("completed: a, b");
  });

  test("in-flight only", () => {
    expect(formatSnapshotSubject([], [{ title: "x" }], false)).toBe(
      "in flight: x",
    );
  });

  test("both completed and in-flight on same row", () => {
    expect(
      formatSnapshotSubject(
        [{ title: "ship A" }],
        [{ title: "still B" }, { title: "still C" }],
        false,
      ),
    ).toBe("completed: ship A · in flight: still B, still C");
  });

  test("first snapshot with no efforts → Initial Snapshot", () => {
    expect(formatSnapshotSubject([], [], true)).toBe("Initial Snapshot");
  });

  test("later snapshot with no efforts → External change", () => {
    expect(formatSnapshotSubject([], [], false)).toBe("External change");
  });

  test("hasOtherBadges suppresses External change fallback", () => {
    expect(formatSnapshotSubject([], [], false, true)).toBe("");
  });

  test("hasOtherBadges does NOT suppress effort labels", () => {
    expect(
      formatSnapshotSubject([{ title: "fix" }], [], false, true),
    ).toBe("completed: fix");
  });

  test("hasOtherBadges does NOT suppress Initial Snapshot", () => {
    expect(formatSnapshotSubject([], [], true, true)).toBe("Initial Snapshot");
  });
});

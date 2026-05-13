import { describe, expect, test } from "bun:test";
import { formatSnapshotSubject } from "./LocalHistoryDashboardPage.js";

describe("formatSnapshotSubject", () => {
  test("efforts win over the isInitial flag", () => {
    expect(formatSnapshotSubject([{ title: "fix snapshot" }], true)).toBe("fix snapshot");
  });

  test("joins multiple effort titles with a middle dot", () => {
    expect(
      formatSnapshotSubject([{ title: "a" }, { title: "b" }], false),
    ).toBe("a · b");
  });

  test("first snapshot with no efforts → Initial Snapshot", () => {
    expect(formatSnapshotSubject([], true)).toBe("Initial Snapshot");
  });

  test("later snapshot with no efforts → External change", () => {
    expect(formatSnapshotSubject([], false)).toBe("External change");
  });
});

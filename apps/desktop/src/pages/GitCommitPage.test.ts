import { describe, expect, test } from "bun:test";
import { buildCommitTitle } from "./GitCommitPage.js";

describe("buildCommitTitle", () => {
  test("uses the first line of the commit subject", () => {
    expect(buildCommitTitle({ sha: "abc1234567", subject: "Fix the thing" })).toBe(
      "abc1234 · Fix the thing",
    );
  });

  test("falls back to '(no message)' when subject is blank", () => {
    expect(buildCommitTitle({ sha: "deadbeef0011", subject: "" })).toBe(
      "deadbee · (no message)",
    );
  });

  test("renders only the SHA prefix when sha is shorter than 7 chars", () => {
    expect(buildCommitTitle({ sha: "abc", subject: "tiny" })).toBe("abc · tiny");
  });
});

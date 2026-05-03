import { describe, expect, test } from "bun:test";
import {
  buildFilePivots,
  diffFunctions,
  fileExtension,
  indexSides,
  isTestPath,
  summarizeTests,
  topDirectory,
} from "./analysisHelpers.js";
import type { BranchChangeEntry } from "../../api-types.js";

describe("isTestPath", () => {
  test("matches common test conventions", () => {
    expect(isTestPath("apps/desktop/src/foo.test.ts")).toBe(true);
    expect(isTestPath("src/bar.spec.tsx")).toBe(true);
    expect(isTestPath("packages/x/tests/helper.ts")).toBe(true);
    expect(isTestPath("crates/foo/src/test_inner.rs")).toBe(true);
    expect(isTestPath("internal/stuff/widget_test.go")).toBe(true);
  });
  test("ignores production files", () => {
    expect(isTestPath("src/foo.ts")).toBe(false);
    expect(isTestPath("crates/oxplow-app/src/lib.rs")).toBe(false);
    expect(isTestPath("apps/desktop/src/api.ts")).toBe(false);
  });
});

describe("path helpers", () => {
  test("fileExtension", () => {
    expect(fileExtension("foo/bar.ts")).toBe("ts");
    expect(fileExtension("README")).toBe("");
    expect(fileExtension(".gitignore")).toBe("");
    expect(fileExtension("a/b/c.tar.gz")).toBe("gz");
  });
  test("topDirectory", () => {
    expect(topDirectory("crates/foo/src/lib.rs")).toBe("crates");
    expect(topDirectory("README.md")).toBe(".");
  });
});

describe("buildFilePivots", () => {
  const files: BranchChangeEntry[] = [
    { path: "src/a.ts", status: "modified", additions: 10, deletions: 2 },
    { path: "src/b.ts", status: "added", additions: 30, deletions: 0 },
    { path: "src/c.rs", status: "deleted", additions: 0, deletions: 15 },
    { path: "docs/readme.md", status: "modified", additions: 5, deletions: 1 },
  ];
  const pivots = buildFilePivots(files);

  test("groups by extension", () => {
    const ts = pivots.byExtension.find((r) => r.key === "ts");
    expect(ts).toBeDefined();
    expect(ts!.files).toBe(2);
    expect(ts!.additions).toBe(40);
    expect(ts!.deletions).toBe(2);
  });
  test("groups by top directory", () => {
    const src = pivots.byTopDir.find((r) => r.key === "src");
    expect(src!.files).toBe(3);
  });
  test("counts by status", () => {
    expect(pivots.byStatus.modified).toBe(2);
    expect(pivots.byStatus.added).toBe(1);
    expect(pivots.byStatus.deleted).toBe(1);
    expect(pivots.byStatus.renamed).toBe(0);
  });
});

describe("diffFunctions", () => {
  const sides = [
    {
      path: "src/foo.ts",
      side: "base",
      functions: [
        { name: "alpha", paramCount: 1, complexity: 3, length: 10, startLine: 1 },
        { name: "beta", paramCount: 2, complexity: 5, length: 20, startLine: 12 },
        { name: "gone", paramCount: 0, complexity: 1, length: 4, startLine: 33 },
      ],
    },
    {
      path: "src/foo.ts",
      side: "head",
      functions: [
        { name: "alpha", paramCount: 1, complexity: 3, length: 10, startLine: 1 }, // unchanged
        { name: "beta", paramCount: 3, complexity: 8, length: 22, startLine: 12 }, // sig + body
        { name: "fresh", paramCount: 1, complexity: 2, length: 6, startLine: 28 }, // added
      ],
    },
  ];
  const buckets = diffFunctions(indexSides(sides));

  test("detects added functions", () => {
    expect(buckets.added.map((f) => f.name)).toEqual(["fresh"]);
  });
  test("detects deleted functions", () => {
    expect(buckets.deleted.map((f) => f.name)).toEqual(["gone"]);
  });
  test("detects signature changes", () => {
    expect(buckets.modifiedSignature).toEqual([
      { path: "src/foo.ts", name: "beta", before: 2, after: 3 },
    ]);
  });
  test("detects body changes alongside signature changes", () => {
    expect(buckets.modifiedBody.map((f) => f.name)).toEqual(["beta"]);
  });
  test("ignores unchanged functions", () => {
    const allChanged = [
      ...buckets.added.map((f) => f.name),
      ...buckets.deleted.map((f) => f.name),
      ...buckets.modifiedBody.map((f) => f.name),
    ];
    expect(allChanged).not.toContain("alpha");
  });
});

describe("summarizeTests", () => {
  const files: BranchChangeEntry[] = [
    { path: "src/foo.ts", status: "modified", additions: 20, deletions: 1 },
    { path: "src/foo.test.ts", status: "modified", additions: 10, deletions: 0 },
    { path: "src/risky.ts", status: "modified", additions: 50, deletions: 0 },
    { path: "docs/x.md", status: "modified", additions: 1, deletions: 0 },
  ];
  const summary = summarizeTests(files);

  test("counts buckets", () => {
    expect(summary.testFiles).toBe(1);
    expect(summary.nonTestFiles).toBe(3);
    expect(summary.modified).toEqual(["src/foo.test.ts"]);
  });
  test("flags risky-untested files when no test in same top dir changed", () => {
    // src/* has a test file change, so risky-untested in src/ are filtered
    // out. docs/* has no companion test.
    expect(summary.riskyUntested.map((r) => r.path)).toEqual(["docs/x.md"]);
  });
  test("ratio", () => {
    expect(summary.ratio).toBeCloseTo(1 / 3);
  });
});

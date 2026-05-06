import { describe, expect, test } from "bun:test";
import {
  attachChurn,
  buildFilePivots,
  diffFunctions,
  fileExtension,
  indexSides,
  isTestFunction,
  isTestPath,
  summarizeTests,
  topDirectory,
  type FunctionChurnRow,
  type FunctionsBuckets,
} from "./analysisHelpers.js";
import { fileInterestingness } from "./interestingness.js";
import type { BranchChangeEntry } from "../../api-types.js";

describe("isTestPath", () => {
  test("matches common test conventions", () => {
    expect(isTestPath("apps/desktop/src/foo.test.ts")).toBe(true);
    expect(isTestPath("src/bar.spec.tsx")).toBe(true);
    expect(isTestPath("packages/x/tests/helper.ts")).toBe(true);
    expect(isTestPath("crates/foo/src/test_inner.rs")).toBe(true);
    expect(isTestPath("internal/stuff/widget_test.go")).toBe(true);
    expect(isTestPath("src/foo/bar_test.clj")).toBe(true);
    expect(isTestPath("src/foo/bar_test.cljc")).toBe(true);
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
        { name: "alpha", paramCount: 1, complexity: 3, length: 10, startLine: 1, containerPath: [], visibility: "public" },
        { name: "beta", paramCount: 2, complexity: 5, length: 20, startLine: 12, containerPath: [], visibility: "public" },
        { name: "gone", paramCount: 0, complexity: 1, length: 4, startLine: 33, containerPath: [], visibility: "public" },
      ],
    },
    {
      path: "src/foo.ts",
      side: "head",
      functions: [
        { name: "alpha", paramCount: 1, complexity: 3, length: 10, startLine: 1, containerPath: [], visibility: "public" }, // unchanged
        { name: "beta", paramCount: 3, complexity: 8, length: 22, startLine: 12, containerPath: [], visibility: "public" }, // sig + body
        { name: "fresh", paramCount: 1, complexity: 2, length: 6, startLine: 28, containerPath: [], visibility: "public" }, // added
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
      { path: "src/foo.ts", name: "beta", containerPath: [], startLine: 12, before: 2, after: 3, visibility: "public" },
    ]);
  });
  test("methods with the same short name in sibling containers don't collide", () => {
    const sidesNested = [
      {
        path: "src/x.ts",
        side: "base",
        functions: [
          { name: "save", paramCount: 0, complexity: 1, length: 4, startLine: 1, containerPath: ["UserStore"], visibility: "public" },
          { name: "save", paramCount: 0, complexity: 1, length: 4, startLine: 10, containerPath: ["DocStore"], visibility: "public" },
        ],
      },
      {
        path: "src/x.ts",
        side: "head",
        functions: [
          { name: "save", paramCount: 1, complexity: 1, length: 4, startLine: 1, containerPath: ["UserStore"], visibility: "public" },
          { name: "save", paramCount: 0, complexity: 1, length: 4, startLine: 10, containerPath: ["DocStore"], visibility: "public" },
        ],
      },
    ];
    const result = diffFunctions(indexSides(sidesNested));
    expect(result.modifiedSignature).toEqual([
      { path: "src/x.ts", name: "save", containerPath: ["UserStore"], startLine: 1, before: 0, after: 1, visibility: "public" },
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

describe("attachChurn", () => {
  test("decorates added + modifiedBody rows by qualified key", () => {
    const buckets: FunctionsBuckets = {
      added: [
        { path: "a.ts", name: "newFn", containerPath: [], startLine: 10, paramCount: 0, complexity: 1, length: 5, visibility: "public" },
      ],
      modifiedBody: [
        { path: "a.ts", name: "existing", containerPath: ["Foo"], startLine: 20, complexityDelta: 1, lengthDelta: 0, visibility: "public" },
        { path: "b.ts", name: "untouched", containerPath: [], startLine: 1, complexityDelta: 0, lengthDelta: 0, visibility: "public" },
      ],
      modifiedSignature: [],
      deleted: [],
    };
    const churn: FunctionChurnRow[] = [
      { path: "a.ts", name: "newFn", containerPath: [], startLineHead: 10, addedLines: 5, deletedLines: 0, modifiedLines: 0 },
      { path: "a.ts", name: "existing", containerPath: ["Foo"], startLineHead: 20, addedLines: 3, deletedLines: 2, modifiedLines: 2 },
    ];
    attachChurn(buckets, churn);
    expect(buckets.added[0]!.churn).toEqual({
      addedLines: 5,
      deletedLines: 0,
      modifiedLines: 0,
      churnPercent: 5 / 10,
    });
    expect(buckets.modifiedBody[0]!.churn?.churnPercent).toBeCloseTo(5 / 10);
    // No churn row for `untouched` → null decoration.
    expect(buckets.modifiedBody[1]!.churn).toBeNull();
  });
});

describe("fileInterestingness", () => {
  const baseFile: BranchChangeEntry = {
    path: "src/calm.ts",
    status: "modified",
    additions: 3,
    deletions: 1,
  };
  const emptyBuckets = { added: [], deleted: [], modifiedSignature: [], modifiedBody: [] };

  test("low score for routine diffs", () => {
    const r = fileInterestingness({ file: baseFile, bucketed: emptyBuckets });
    expect(r.score).toBeLessThan(4);
    expect(r.reasons).toEqual([]);
  });

  test("complexity spike pushes score up and adds a reason", () => {
    const r = fileInterestingness({
      file: { path: "src/x.ts", status: "modified", additions: 50, deletions: 30 },
      bucketed: {
        added: [],
        deleted: [],
        modifiedSignature: [],
        modifiedBody: [
          { path: "src/x.ts", name: "f", containerPath: [], startLine: 1, complexityDelta: 8, lengthDelta: 10, visibility: "public" },
        ],
      },
    });
    expect(r.score).toBeGreaterThan(15);
    expect(r.reasons.some((s) => s.startsWith("complexity"))).toBe(true);
  });

  test("long new function bumps score", () => {
    const longFn = fileInterestingness({
      file: { path: "src/x.ts", status: "modified", additions: 100, deletions: 0 },
      bucketed: {
        added: [
          { path: "src/x.ts", name: "big", containerPath: [], startLine: 1, paramCount: 0, complexity: 5, length: 180, visibility: "public" },
        ],
        deleted: [],
        modifiedSignature: [],
        modifiedBody: [],
      },
    });
    expect(longFn.reasons.some((s) => s.includes("180-line"))).toBe(true);
  });
});

describe("isTestFunction", () => {
  test("classifies any function in a test file as a test", () => {
    expect(isTestFunction("apps/desktop/src/foo.test.ts", "anything")).toBe(true);
    expect(isTestFunction("crates/foo/tests/bar.rs", "helper")).toBe(true);
    expect(isTestFunction("internal/mod_test.go", "anything")).toBe(true);
  });
  test("classifies by Python / Rust test_* convention", () => {
    expect(isTestFunction("crates/foo/src/lib.rs", "test_thing")).toBe(true);
    expect(isTestFunction("scripts/x.py", "test_run")).toBe(true);
  });
  test("classifies by Go Test / Benchmark / Example prefix", () => {
    expect(isTestFunction("internal/x.go", "TestSomething")).toBe(true);
    expect(isTestFunction("internal/x.go", "BenchmarkParse")).toBe(true);
    expect(isTestFunction("internal/x.go", "ExampleUsage")).toBe(true);
  });
  test("does not classify production code as tests", () => {
    expect(isTestFunction("crates/foo/src/lib.rs", "process")).toBe(false);
    expect(isTestFunction("apps/desktop/src/foo.ts", "Tester")).toBe(false); // 'Tester' alone shouldn't match Test[A-Z]
    expect(isTestFunction("apps/desktop/src/foo.ts", "tested")).toBe(false);
  });
  test("classifies functions inside Rust mod tests / FooTests classes", () => {
    // Rust idiom: #[cfg(test)] mod tests { #[test] fn parses() { ... } }
    expect(isTestFunction("crates/foo/src/lib.rs", "parses_simple_hunk", ["tests"])).toBe(true);
    // Java/JS class-style test container
    expect(isTestFunction("src/Foo.java", "validates", ["FooTests"])).toBe(true);
    expect(isTestFunction("src/Foo.java", "validates", ["FooTest"])).toBe(true);
    // Clojure ns convention: foo.bar-test
    expect(isTestFunction("src/foo/bar_test.clj", "round-trip", ["foo.bar-test"])).toBe(true);
    // Production sibling stays production
    expect(isTestFunction("crates/foo/src/lib.rs", "helper", ["impl_block"])).toBe(false);
  });
});

import { summarizeTestFunctions } from "./analysisHelpers.js";

describe("summarizeTestFunctions", () => {
  test("counts added / deleted / modified test functions; ignores production fns", () => {
    const buckets: FunctionsBuckets = {
      added: [
        { path: "lib.rs", name: "test_one", containerPath: ["tests"], startLine: 1, paramCount: 0, complexity: 1, length: 3, visibility: "public" },
        { path: "lib.rs", name: "do_thing", containerPath: [], startLine: 1, paramCount: 0, complexity: 1, length: 3, visibility: "public" },
      ],
      deleted: [
        { path: "x.go", name: "TestOld", containerPath: [], startLine: 1, visibility: "public" },
      ],
      modifiedSignature: [
        { path: "lib.rs", name: "parses", containerPath: ["tests"], startLine: 5, before: 0, after: 1, visibility: "public" },
      ],
      modifiedBody: [
        { path: "lib.rs", name: "parses", containerPath: ["tests"], startLine: 5, complexityDelta: 1, lengthDelta: 0, visibility: "public" },
        { path: "lib.rs", name: "process", containerPath: [], startLine: 100, complexityDelta: 2, lengthDelta: 0, visibility: "public" },
      ],
    };
    const counts = summarizeTestFunctions(buckets);
    expect(counts.added).toBe(1); // test_one (do_thing is production)
    expect(counts.deleted).toBe(1); // TestOld
    expect(counts.modified).toBe(1); // parses appears in both sig+body but counted once
  });

  test("zero counts when no test functions changed", () => {
    const buckets: FunctionsBuckets = {
      added: [],
      deleted: [],
      modifiedSignature: [],
      modifiedBody: [
        { path: "lib.rs", name: "process", containerPath: [], startLine: 1, complexityDelta: 1, lengthDelta: 0, visibility: "public" },
      ],
    };
    const counts = summarizeTestFunctions(buckets);
    expect(counts).toEqual({ added: 0, modified: 0, deleted: 0 });
  });
});

import { summarizeTestLineRatio } from "./analysisHelpers.js";

describe("summarizeTestLineRatio", () => {
  test("ratio = test churn / production churn (added + deleted)", () => {
    const churn: FunctionChurnRow[] = [
      // production
      { path: "lib.rs", name: "process", containerPath: [], startLineHead: 1, addedLines: 10, deletedLines: 4, modifiedLines: 4 },
      // test (Rust mod tests)
      { path: "lib.rs", name: "parses", containerPath: ["tests"], startLineHead: 30, addedLines: 6, deletedLines: 2, modifiedLines: 2 },
      // test (test_ prefix)
      { path: "scripts/x.py", name: "test_run", containerPath: [], startLineHead: 1, addedLines: 1, deletedLines: 0, modifiedLines: 0 },
    ];
    const result = summarizeTestLineRatio(churn);
    expect(result.testLines).toBe(9); // 6+2 + 1
    expect(result.productionLines).toBe(14); // 10 + 4
    expect(result.ratio).toBeCloseTo(9 / 14);
  });

  test("zero production lines yields ratio 0 (no div-by-zero)", () => {
    const churn: FunctionChurnRow[] = [
      { path: "lib.rs", name: "test_x", containerPath: [], startLineHead: 1, addedLines: 5, deletedLines: 0, modifiedLines: 0 },
    ];
    const result = summarizeTestLineRatio(churn);
    expect(result.testLines).toBe(5);
    expect(result.productionLines).toBe(0);
    expect(result.ratio).toBe(0);
  });
});

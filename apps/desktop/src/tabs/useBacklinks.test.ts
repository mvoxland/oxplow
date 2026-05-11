import { describe, expect, test } from "bun:test";

import { canonicalIdForTarget } from "./useBacklinks.js";
import {
  directoryRef,
  fileRef,
  findingRef,
  gitCommitRef,
  taskRef,
  wikiPageRef,
} from "./pageRefs.js";

describe("canonicalIdForTarget", () => {
  test("taskRef returns the integer id as a string", () => {
    // Regression: taskRef stores itemId as a number, but Tauri
    // commands take String — without the stringify, IPC throws and
    // the Outbound dropdown on a task page silently renders empty.
    const ref = taskRef(42);
    const id = canonicalIdForTarget(ref);
    expect(id).toBe("42");
    expect(typeof id).toBe("string");
  });

  test("wikiPageRef returns slug", () => {
    expect(canonicalIdForTarget(wikiPageRef("url-schemes"))).toBe("url-schemes");
  });

  test("fileRef returns the workspace-relative path", () => {
    expect(canonicalIdForTarget(fileRef("src/app.ts"))).toBe("src/app.ts");
  });

  test("directoryRef returns the path", () => {
    expect(canonicalIdForTarget(directoryRef("src/components"))).toBe("src/components");
  });

  test("gitCommitRef returns the sha", () => {
    expect(canonicalIdForTarget(gitCommitRef("abc1234"))).toBe("abc1234");
  });

  test("findingRef returns the finding id", () => {
    expect(canonicalIdForTarget(findingRef("fnd-1"))).toBe("fnd-1");
  });

  test("untracked kinds return null", () => {
    expect(canonicalIdForTarget({ id: "settings", kind: "settings", payload: null })).toBeNull();
  });
});

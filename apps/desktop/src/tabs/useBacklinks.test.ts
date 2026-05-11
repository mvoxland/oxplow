import { describe, expect, test } from "bun:test";

import { canonicalIdForTarget, dedupeEntriesByTarget } from "./useBacklinks.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
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

describe("dedupeEntriesByTarget", () => {
  test("collapses wiki + on-disk file shadow into a single wiki row", () => {
    const entries: BacklinkEntry[] = [
      { ref: fileRef(".oxplow/wiki/local-snapshots.md"), label: ".oxplow/wiki/local-snapshots.md", subtitle: "touched" },
      { ref: wikiPageRef("local-snapshots"), label: "Local Snapshots", subtitle: "impact (created)" },
      { ref: wikiPageRef("local-snapshots"), label: "Local Snapshots", subtitle: "wiki link" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].ref.kind).toBe("wiki");
    expect(out[0].label).toBe("Local Snapshots");
    // Subtitles merged in first-seen order, deduped.
    expect(out[0].subtitle).toBe("touched · impact (created) · wiki link");
  });

  test("preserves rows that point at different targets", () => {
    const entries: BacklinkEntry[] = [
      { ref: wikiPageRef("a"), label: "A", subtitle: "wiki link" },
      { ref: wikiPageRef("b"), label: "B", subtitle: "wiki link" },
      { ref: fileRef("src/x.ts"), label: "src/x.ts", subtitle: "touched" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(3);
  });

  test("collapses two ref_types pointing at the same wiki page", () => {
    const entries: BacklinkEntry[] = [
      { ref: wikiPageRef("url-schemes"), label: "URL Schemes", subtitle: "impact (created)" },
      { ref: wikiPageRef("url-schemes"), label: "URL Schemes", subtitle: "wiki link" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].subtitle).toBe("impact (created) · wiki link");
  });

  test("file path that doesn't look like a wiki shadow stays a file row", () => {
    const entries: BacklinkEntry[] = [
      { ref: fileRef("src/foo.rs"), label: "src/foo.rs", subtitle: "touched" },
      { ref: fileRef("src/foo.rs"), label: "src/foo.rs", subtitle: "wiki link" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].ref.kind).toBe("file");
    expect(out[0].subtitle).toBe("touched · wiki link");
  });

  test("drops blank subtitles cleanly", () => {
    const entries: BacklinkEntry[] = [
      { ref: wikiPageRef("a"), label: "A", subtitle: "" },
      { ref: wikiPageRef("a"), label: "A", subtitle: "wiki link" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out[0].subtitle).toBe("wiki link");
  });
});

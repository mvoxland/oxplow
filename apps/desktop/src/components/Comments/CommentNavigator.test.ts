import { describe, expect, test } from "bun:test";
import type { CommentThread } from "../../tauri-bridge/generated/bindings.js";
import { partitionPageComments, stepComment } from "./pageCommentNav.js";

const thread = (id: number, orphaned: boolean): CommentThread =>
  ({ comment: { id, orphaned }, messages: [] }) as unknown as CommentThread;

describe("partitionPageComments", () => {
  test("splits jumpable (anchored) from orphaned and counts the total", () => {
    const r = partitionPageComments([thread(1, false), thread(2, true), thread(3, false)]);
    expect(r.jumpable.map((t) => t.comment.id)).toEqual([1, 3]);
    expect(r.orphaned.map((t) => t.comment.id)).toEqual([2]);
    expect(r.total).toBe(3);
  });

  test("empty input", () => {
    expect(partitionPageComments([])).toEqual({ jumpable: [], orphaned: [], total: 0 });
  });
});

describe("stepComment", () => {
  const threads = [thread(1, false), thread(2, true), thread(3, false), thread(4, false)];
  // jumpable order: [1, 3, 4]

  test("next cycles through jumpable comments", () => {
    expect(stepComment(threads, 1, 1)).toBe(3);
    expect(stepComment(threads, 3, 1)).toBe(4);
    expect(stepComment(threads, 4, 1)).toBe(1); // wrap
  });

  test("prev cycles backward", () => {
    expect(stepComment(threads, 1, -1)).toBe(4); // wrap
    expect(stepComment(threads, 4, -1)).toBe(3);
  });

  test("from an orphaned/unknown current, enters from the matching edge", () => {
    expect(stepComment(threads, 2, 1)).toBe(1); // forward → first
    expect(stepComment(threads, 2, -1)).toBe(4); // back → last
  });

  test("null when there's nowhere to step", () => {
    expect(stepComment([thread(1, false)], 1, 1)).toBeNull();
    expect(stepComment([thread(1, true)], 1, 1)).toBeNull();
    expect(stepComment([], 1, 1)).toBeNull();
  });
});

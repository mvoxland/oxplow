import { afterEach, describe, expect, test } from "bun:test";
import {
  clearCommentReveal,
  peekPendingCommentReveal,
  requestCommentReveal,
  subscribeCommentReveal,
} from "./comment-reveal-bus.js";

afterEach(() => {
  // Drain any pending reveal so tests don't leak into each other.
  const id = peekPendingCommentReveal();
  if (id != null) clearCommentReveal(id);
});

describe("comment-reveal-bus", () => {
  test("requesting a reveal makes it the pending id", () => {
    expect(peekPendingCommentReveal()).toBeNull();
    requestCommentReveal(42);
    expect(peekPendingCommentReveal()).toBe(42);
  });

  test("subscribers are notified on each request", () => {
    let hits = 0;
    const unsub = subscribeCommentReveal(() => {
      hits += 1;
    });
    requestCommentReveal(1);
    requestCommentReveal(2);
    expect(hits).toBe(2);
    unsub();
    requestCommentReveal(3);
    expect(hits).toBe(2);
  });

  test("clear only drops the matching pending id", () => {
    requestCommentReveal(7);
    clearCommentReveal(99); // non-matching: no-op
    expect(peekPendingCommentReveal()).toBe(7);
    clearCommentReveal(7);
    expect(peekPendingCommentReveal()).toBeNull();
  });
});

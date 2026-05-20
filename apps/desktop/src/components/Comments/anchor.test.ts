import { describe, expect, it } from "bun:test";

import { reanchor, resolveQuoteOffset } from "./anchor.js";

describe("resolveQuoteOffset", () => {
  it("returns null for an empty quote", () => {
    expect(resolveQuoteOffset("hello world", "")).toBeNull();
  });

  it("finds a unique quote", () => {
    expect(resolveQuoteOffset("the quick brown fox", "brown")).toBe(10);
  });

  it("returns null when the quote is gone (orphaned)", () => {
    expect(resolveQuoteOffset("the quick fox", "brown")).toBeNull();
  });

  it("returns the first occurrence with no hint", () => {
    expect(resolveQuoteOffset("ab ab ab", "ab")).toBe(0);
  });

  it("disambiguates duplicates by proximity to the hint", () => {
    // "ab" at 0, 3, 6 — hint near 6 should pick 6.
    expect(resolveQuoteOffset("ab ab ab", "ab", 6)).toBe(6);
    expect(resolveQuoteOffset("ab ab ab", "ab", 3)).toBe(3);
  });
});

describe("reanchor", () => {
  it("carries the quote length and a found offset", () => {
    expect(reanchor("the quick brown fox", "brown")).toEqual({ offset: 10, length: 5 });
  });

  it("reports a null offset for an orphaned quote", () => {
    expect(reanchor("nothing here", "brown")).toEqual({ offset: null, length: 5 });
  });
});

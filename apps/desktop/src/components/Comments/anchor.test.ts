import { describe, expect, it } from "bun:test";

import {
  extractContext,
  fuzzySubstring,
  reanchor,
  resolveAnchor,
  resolveQuoteOffset,
} from "./anchor.js";

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

describe("extractContext", () => {
  it("captures up to CONTEXT_LEN chars each side, truncated at edges", () => {
    expect(extractContext("hello world", 6, 11)).toEqual({ prefix: "hello ", suffix: "" });
    expect(extractContext("hello world", 0, 5)).toEqual({ prefix: "", suffix: " world" });
  });
});

describe("fuzzySubstring", () => {
  it("finds an exact substring at distance 0", () => {
    expect(fuzzySubstring("cat", "the cat sat", 1)).toEqual({ start: 4, end: 7, dist: 0 });
  });

  it("tolerates one substitution within maxDist", () => {
    expect(fuzzySubstring("cat", "the cot sat", 1)).toEqual({ start: 4, end: 7, dist: 1 });
  });

  it("returns null when nothing is within maxDist", () => {
    expect(fuzzySubstring("xyz", "the dog sat", 1)).toBeNull();
  });
});

describe("resolveAnchor", () => {
  it("resolves a unique exact quote", () => {
    expect(resolveAnchor("the quick brown fox", { quote: "brown" })).toEqual({
      offset: 10,
      length: 5,
      confidence: "exact",
    });
  });

  it("treats an empty quote as orphaned", () => {
    expect(resolveAnchor("anything", { quote: "" })).toEqual({
      offset: null,
      length: 0,
      confidence: "none",
    });
  });

  it("disambiguates exact duplicates by proximity when no context", () => {
    expect(resolveAnchor("ab ab ab", { quote: "ab", hintOffset: 6 })).toEqual({
      offset: 6,
      length: 2,
      confidence: "exact",
    });
  });

  it("disambiguates exact duplicates by prefix/suffix context", () => {
    // Two "X" occurrences (offsets 3 and 11); context points at the 2nd.
    const text = "aa X bb\ncc X dd";
    const r = resolveAnchor(text, { quote: "X", prefix: "cc ", suffix: " dd" });
    expect(r).toEqual({ offset: 11, length: 1, confidence: "exact" });
  });

  it("fuzzy-matches a quote with a small edit", () => {
    // doc says "calculate", quote captured "calcuate" (dropped an l).
    const text = "please calculate total now";
    const r = resolveAnchor(text, { quote: "calcuate total" });
    expect(r.confidence).toBe("fuzzy");
    expect(r.offset).toBe(7); // start of "calculate"
  });

  it("orphans when the quote changed beyond the threshold", () => {
    expect(resolveAnchor("the quick fox", { quote: "brown" })).toEqual({
      offset: null,
      length: 5,
      confidence: "none",
    });
  });

  it("never fuzzy-matches a short quote without context", () => {
    expect(resolveAnchor("the cat", { quote: "dog" }).confidence).toBe("none");
  });

  it("bounds the fuzzy search to a window around the hint", () => {
    // The only near-match sits far past the window anchored at hint 0.
    const text = "z".repeat(200) + "please calculate total";
    expect(resolveAnchor(text, { quote: "calcuate total", hintOffset: 0 }).confidence).toBe("none");
  });
});

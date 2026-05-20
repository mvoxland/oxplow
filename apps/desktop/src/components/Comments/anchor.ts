//! Resilient re-anchoring shared across comment surfaces.
//!
//! A comment's `quote` (the originally-selected text) is the durable
//! anchor; the stored position is only a hint. On load each surface
//! re-locates the quote in the *current* content and, if it can't,
//! marks the comment orphaned. The core search is surface-agnostic —
//! it works on a plain string — so rich-text (ProseMirror) and code
//! (Monaco) integrations both reuse it after flattening their content
//! to text.

/// Locate `quote` in `text`, preferring the occurrence whose start is
/// closest to `hintOffset` when there are several. Returns the start
/// offset, or `null` when the quote no longer appears (→ orphaned).
export function resolveQuoteOffset(
  text: string,
  quote: string,
  hintOffset?: number,
): number | null {
  if (quote.length === 0) return null;

  // Collect every occurrence. Quotes are usually short and unique, so
  // this is cheap; bail early on the common single-match case.
  const offsets: number[] = [];
  let from = 0;
  for (;;) {
    const idx = text.indexOf(quote, from);
    if (idx === -1) break;
    offsets.push(idx);
    from = idx + 1;
  }

  if (offsets.length === 0) return null;
  if (offsets.length === 1 || hintOffset === undefined) return offsets[0];

  // Disambiguate by proximity to the hint — content edits move the
  // quote, but rarely far, so the nearest match is the right one.
  let best = offsets[0];
  let bestDist = Math.abs(best - hintOffset);
  for (const off of offsets.slice(1)) {
    const dist = Math.abs(off - hintOffset);
    if (dist < bestDist) {
      best = off;
      bestDist = dist;
    }
  }
  return best;
}

/// A resolved text-offset anchor. `null` offset means orphaned.
export interface ResolvedAnchor {
  offset: number | null;
  length: number;
}

/// Re-resolve a quote against current text, returning the new offset
/// (or orphaned) and the quote length. Surfaces convert `offset` into
/// their own coordinate space (ProseMirror positions, Monaco ranges).
export function reanchor(
  text: string,
  quote: string,
  hintOffset?: number,
): ResolvedAnchor {
  return { offset: resolveQuoteOffset(text, quote, hintOffset), length: quote.length };
}

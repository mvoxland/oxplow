import { Extension } from "@tiptap/react";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import type { Node as PMNode } from "@tiptap/pm/model";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

import { resolveQuoteOffset } from "../Comments/anchor.js";

/// A comment's resolved span in ProseMirror coordinates.
export interface CommentRange {
  id: number;
  from: number;
  to: number;
}

export const commentDecorationsKey = new PluginKey<DecorationSet>("commentDecorations");

/// ProseMirror plugin that paints inline highlights for the supplied
/// comment ranges. Critically these are **decorations, not stored
/// marks** — they overlay the document without touching the markdown
/// that round-trips to disk. Ranges are pushed in from React via a
/// transaction meta (`commentDecorationsKey`); between pushes the set
/// maps through doc edits so highlights track typing.
export const CommentDecorations = Extension.create<{
  onClickComment: ((id: number, rect: DOMRect) => void) | null;
}>({
  name: "commentDecorations",

  addOptions() {
    return { onClickComment: null };
  },

  addProseMirrorPlugins() {
    const options = this.options;
    return [
      new Plugin<DecorationSet>({
        key: commentDecorationsKey,
        state: {
          init() {
            return DecorationSet.empty;
          },
          apply(tr, old) {
            const next = tr.getMeta(commentDecorationsKey) as CommentRange[] | undefined;
            if (next) {
              const decos = next
                .filter((r) => r.from < r.to)
                .map((r) =>
                  Decoration.inline(r.from, r.to, {
                    class: "oxplow-comment-highlight",
                    "data-comment-id": String(r.id),
                  }),
                );
              return DecorationSet.create(tr.doc, decos);
            }
            return old.map(tr.mapping, tr.doc);
          },
        },
        props: {
          decorations(state) {
            return commentDecorationsKey.getState(state);
          },
          handleClick(_view, _pos, event) {
            const target = event.target as HTMLElement | null;
            const el = target?.closest?.("[data-comment-id]") as HTMLElement | null;
            if (el && options.onClickComment) {
              options.onClickComment(Number(el.getAttribute("data-comment-id")), el.getBoundingClientRect());
              return true;
            }
            return false;
          },
        },
      }),
    ];
  },
});

/// Flatten a doc's text nodes into a single string plus an offset→pos
/// map, so a plain-text quote search can be mapped back to ProseMirror
/// document positions. Block boundaries are not represented (no
/// separator), so a quote spanning two blocks won't match — those
/// comments orphan, which is acceptable for v1.
function flatten(doc: PMNode): { text: string; map: number[] } {
  let text = "";
  const map: number[] = []; // map[textOffset] = doc pos of that char
  doc.descendants((node, pos) => {
    if (node.isText && node.text) {
      for (let i = 0; i < node.text.length; i++) {
        map.push(pos + i);
      }
      text += node.text;
    }
    return true;
  });
  return { text, map };
}

/// Re-resolve a quote to a `{ from, to }` doc range. Tries the stored
/// hint range first (fast path when nothing moved), then a proximity
/// search. Returns `null` when the quote no longer appears → orphaned.
export function findCommentRange(
  doc: PMNode,
  quote: string,
  hintFrom?: number,
  hintTo?: number,
): { from: number; to: number } | null {
  if (quote.length === 0) return null;

  // Fast path: the hint range still spells the quote exactly.
  if (
    hintFrom !== undefined &&
    hintTo !== undefined &&
    hintFrom >= 0 &&
    hintTo <= doc.content.size &&
    hintFrom < hintTo
  ) {
    if (doc.textBetween(hintFrom, hintTo) === quote) {
      return { from: hintFrom, to: hintTo };
    }
  }

  const { text, map } = flatten(doc);
  // Translate the hint's doc pos into a text offset for proximity.
  const hintOffset =
    hintFrom !== undefined ? map.findIndex((p) => p >= hintFrom) : undefined;
  const offset = resolveQuoteOffset(text, quote, hintOffset === -1 ? undefined : hintOffset);
  if (offset === null) return null;
  const startPos = map[offset];
  const endPos = map[offset + quote.length - 1];
  if (startPos === undefined || endPos === undefined) return null;
  return { from: startPos, to: endPos + 1 };
}

import { useEffect, useRef, useState } from "react";
import type { CSSProperties, MouseEvent as ReactMouseEvent } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown } from "tiptap-markdown";
import { Pencil } from "lucide-react";
import { InternalLink } from "./InternalLink.js";
import { MermaidBlock } from "./MermaidBlock.js";
import {
  CommentDecorations,
  commentDecorationsKey,
  findCommentRange,
  type CommentRange,
} from "./CommentDecorations.js";
import { createComment, setCommentAnchor } from "../../api.js";
import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { useCommentsForTarget } from "../Comments/useCommentsForTarget.js";
import { CommentPopover } from "../Comments/CommentPopover.js";
import { NewCommentPopover } from "../Comments/NewCommentPopover.js";
import { ContextMenu } from "../ContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { parseMarkdownLink } from "../Wiki/MarkdownView.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef, directoryRef, gitCommitRef, wikiPageRef } from "../../tabs/pageRefs.js";
import { DISK } from "../../file-version.js";

/// Comment integration bundle. When provided, the field highlights
/// anchored ranges and exposes "Add comment" / "Open comment" via the
/// right-click menu (no auto-popping button or click-to-open, so
/// comments don't fight selection/cursor). `targetId` identifies the
/// page (wiki slug / task id); `streamId` is the hard scope and
/// `threadId` the origin thread (null for non-thread-bound surfaces).
export interface RichTextCommentConfig {
  streamId: string;
  threadId: string | null;
  targetKind: string;
  targetId: string;
  author?: string;
}

interface PendingSelection {
  quote: string;
  from: number;
  to: number;
  rect: DOMRect;
}

/**
 * Shared rich-text editor surface. One instance per editable region
 * (title saves to one field, description to another, etc.) — the page
 * composes them at the React level.
 *
 * Storage stays markdown. tiptap-markdown handles GFM round-trip on
 * mount and on save; the `MermaidBlock` NodeView paints rendered SVG
 * over the editable fenced code, so users see the diagram unless they
 * click into it.
 *
 * Save model: debounced 300ms while typing, and immediate on blur. The
 * `onCommit` callback is responsible for the actual persistence.
 *
 * Pencil affordance: a small `Pencil` icon sits in the top-right of
 * the editor surface, opacity ~0.4 by default, full opacity on hover
 * or focus. Read-only blocks elsewhere on the page must not show this
 * — that's the visual signal "this is for reading."
 */
export interface RichTextFieldProps {
  value: string;
  onCommit: (markdown: string) => void;
  placeholder?: string;
  /** Disable headings/blocks for inline-only fields (e.g. a wiki page
   *  title). Default false. */
  inlineOnly?: boolean;
  /** Optional className applied to the wrapper. */
  className?: string;
  style?: CSSProperties;
  /** When true, no pencil affordance (e.g. effort summaries — but
   *  those should use MarkdownView, not this field). Default false. */
  hidePencil?: boolean;
  /** When set, the field becomes comment-enabled (highlights, the
   *  selection affordance, and the thread popover). */
  comments?: RichTextCommentConfig;
}

export function RichTextField({
  value,
  onCommit,
  placeholder,
  inlineOnly = false,
  className,
  style,
  hidePencil,
  comments,
}: RichTextFieldProps) {
  const lastCommittedRef = useRef(value);
  const debounceRef = useRef<number | null>(null);

  // Comment state. The hook is always called (empty target → no fetch).
  const { threads } = useCommentsForTarget(comments?.targetKind ?? "", comments?.targetId ?? "");
  const [activeComment, setActiveComment] = useState<{ id: number; rect: DOMRect } | null>(null);
  const [pendingSel, setPendingSel] = useState<PendingSelection | null>(null);
  const [commentMenu, setCommentMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(
    null,
  );

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // Replaced by MermaidBlock (which `extend`s CodeBlock under the
        // same name "codeBlock"). Avoid the duplicate name warning.
        codeBlock: false,
        // Inline-only fields skip block features at the schema level.
        heading: inlineOnly ? false : undefined,
        bulletList: inlineOnly ? false : undefined,
        orderedList: inlineOnly ? false : undefined,
        blockquote: inlineOnly ? false : undefined,
        horizontalRule: inlineOnly ? false : undefined,
      }),
      MermaidBlock,
      InternalLink,
      // Decorations only — opening is via the right-click menu, not click.
      CommentDecorations.configure({ onClickComment: null }),
      Placeholder.configure({ placeholder: placeholder ?? "" }),
      Markdown.configure({
        html: false,
        linkify: false,
        breaks: false,
        transformPastedText: true,
        transformCopiedText: false,
      }),
    ],
    content: value,
    editorProps: {
      attributes: {
        class: "oxplow-md oxplow-rt-editor",
      },
    },
    onUpdate({ editor }) {
      if (debounceRef.current != null) window.clearTimeout(debounceRef.current);
      debounceRef.current = window.setTimeout(() => {
        const md = editor.storage.markdown?.getMarkdown?.() ?? "";
        if (md !== lastCommittedRef.current) {
          lastCommittedRef.current = md;
          onCommit(md);
        }
      }, 300);
    },
    onBlur({ editor }) {
      if (debounceRef.current != null) {
        window.clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
      const md = editor.storage.markdown?.getMarkdown?.() ?? "";
      if (md !== lastCommittedRef.current) {
        lastCommittedRef.current = md;
        onCommit(md);
      }
    },
  });

  // Keep the editor in sync when the upstream value changes from
  // outside (e.g. another tab edited the same task). Don't clobber
  // the user's in-progress typing — skip the sync while the editor
  // has focus.
  useEffect(() => {
    if (!editor) return;
    if (editor.isFocused) return;
    if (value === lastCommittedRef.current) return;
    lastCommittedRef.current = value;
    editor.commands.setContent(value, false);
  }, [editor, value]);

  // On unmount, flush any pending debounce.
  useEffect(() => {
    return () => {
      if (debounceRef.current != null) {
        window.clearTimeout(debounceRef.current);
      }
    };
  }, []);

  // Re-anchor each comment's quote against the current doc and push the
  // resolved ranges into the decoration plugin. Recomputes when the
  // thread list changes or the document content is re-synced; live
  // typing in between is handled by the plugin mapping its set forward.
  // A corrected/orphaned anchor is persisted via `setCommentAnchor`,
  // which deliberately emits no event so this doesn't loop.
  useEffect(() => {
    if (!editor || !comments) return;
    const doc = editor.state.doc;
    const ranges: CommentRange[] = [];
    for (const thread of threads) {
      const c = thread.comment;
      let hintFrom: number | undefined;
      let hintTo: number | undefined;
      try {
        const parsed = JSON.parse(c.anchor_json) as { from?: number; to?: number };
        hintFrom = typeof parsed.from === "number" ? parsed.from : undefined;
        hintTo = typeof parsed.to === "number" ? parsed.to : undefined;
      } catch {
        // Malformed hint — fall back to a pure quote search.
      }
      const range = findCommentRange(doc, c.quote, hintFrom, hintTo);
      if (range) {
        ranges.push({ id: c.id, from: range.from, to: range.to });
        if (c.orphaned || hintFrom !== range.from || hintTo !== range.to) {
          void setCommentAnchor(c.id, JSON.stringify({ from: range.from, to: range.to }), false);
        }
      } else if (!c.orphaned) {
        void setCommentAnchor(c.id, c.anchor_json, true);
      }
    }
    editor.view.dispatch(editor.state.tr.setMeta(commentDecorationsKey, ranges));
  }, [editor, threads, comments, value]);

  // Open the new-comment composer anchored to the current selection.
  // Anchors to the caret at the END of the selection (coordsAtPos), not
  // the selection's bounding box — a multi-line / mid-paragraph box-left
  // lands the composer far from where the user is looking.
  const startCommentForSelection = () => {
    if (!editor || !comments) return;
    const { from, to, empty } = editor.state.selection;
    if (empty) return;
    const quote = editor.state.doc.textBetween(from, to).trim();
    if (!quote) return;
    let rect: DOMRect;
    try {
      const c = editor.view.coordsAtPos(to);
      rect = new DOMRect(c.left, c.top, 0, c.bottom - c.top);
    } catch {
      const domSel = window.getSelection();
      if (!domSel || domSel.rangeCount === 0) return;
      rect = domSel.getRangeAt(0).getBoundingClientRect();
    }
    setActiveComment(null);
    setPendingSel({ quote, from, to, rect });
  };

  // Right-click menu. Always shown (the native webview menu is never
  // allowed in our editor); carries Cut/Copy/Paste plus, when
  // comment-enabled, "Add Comment" (on a selection) / "Open Comment" (on
  // a commented range). Clipboard ops use positions captured here at
  // menu-open so they survive the menu click moving focus.
  const handleContextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (!editor) return;
    event.preventDefault();
    const { from, to, empty } = editor.state.selection;
    const text = empty ? "" : editor.state.doc.textBetween(from, to);
    const targetEl = event.target as HTMLElement | null;
    const commentEl = comments
      ? (targetEl?.closest?.("[data-comment-id]") as HTMLElement | null)
      : null;
    const commentId = commentEl ? Number(commentEl.getAttribute("data-comment-id")) : null;

    const items: MenuItem[] = [
      {
        id: "cut",
        label: "Cut",
        enabled: !empty,
        run: async () => {
          if (text) await navigator.clipboard.writeText(text);
          editor.chain().focus().deleteRange({ from, to }).run();
        },
      },
      {
        id: "copy",
        label: "Copy",
        enabled: !empty,
        run: () => {
          if (text) void navigator.clipboard.writeText(text);
        },
      },
      {
        id: "paste",
        label: "Paste",
        enabled: true,
        run: async () => {
          const t = await navigator.clipboard.readText();
          if (t) editor.chain().focus().insertContentAt(empty ? from : { from, to }, t).run();
        },
      },
    ];
    if (comments && !empty) {
      items.push({
        id: "comment.add",
        label: "Add Comment",
        enabled: true,
        run: () => startCommentForSelection(),
      });
    }
    if (comments && commentId != null) {
      const rect = commentEl!.getBoundingClientRect();
      items.push({
        id: "comment.open",
        label: "Open Comment",
        enabled: true,
        run: () => {
          setPendingSel(null);
          setActiveComment({ id: commentId, rect });
        },
      });
    }
    setCommentMenu({ x: event.clientX, y: event.clientY, items });
  };

  const handleCreateComment = async (input: { body: string; intent: CommentIntent }) => {
    if (!comments || !pendingSel) return;
    await createComment({
      streamId: comments.streamId,
      threadId: comments.threadId,
      targetKind: comments.targetKind,
      targetId: comments.targetId,
      quote: pendingSel.quote,
      anchorJson: JSON.stringify({ from: pendingSel.from, to: pendingSel.to }),
      intent: input.intent,
      author: comments.author ?? "user",
      body: input.body,
    });
    setPendingSel(null);
  };

  const activeThread =
    activeComment != null ? threads.find((t) => t.comment.id === activeComment.id) : undefined;

  const wrapperStyle: CSSProperties = {
    position: "relative",
    padding: "6px 8px",
    borderRadius: 6,
    transition: "background-color 120ms ease",
    ...style,
  };

  // Plain-click on a wikilink / file: / dir: / gitcommit: anchor inside
  // the editable surface should follow the link, not place a cursor.
  // Mirrors `MarkdownView`'s click semantics so the read-only and
  // editable surfaces feel the same: in-tab navigate via
  // `PageNavigationContext`, modifier/middle/right click escapes to a
  // new tab. Cursor placement inside link text is sacrificed — arrow
  // in from adjacent text — which is fine for wikilinks since the
  // visible label is rarely the cursor target.
  const ctxNav = useOptionalPageNavigation();
  const handleAnchorIntent = (event: ReactMouseEvent<HTMLDivElement>, isAux: boolean): boolean => {
    const target = event.target as HTMLElement | null;
    const anchor = target?.closest?.("a");
    if (!anchor) return false;
    const href = anchor.getAttribute("href") ?? "";
    const parsed = parseMarkdownLink(href);
    if (parsed.kind === "anchor" || parsed.kind === "empty") return false;
    event.preventDefault();
    event.stopPropagation();
    const newTab = isAux || event.metaKey || event.ctrlKey || event.button === 1;
    if (parsed.kind === "external") {
      window.open(href, "_blank", "noopener,noreferrer");
      return true;
    }
    if (parsed.kind === "file") {
      const version = parsed.version ?? DISK;
      ctxNav?.navigate(fileRef(parsed.path, version), { newTab });
      return true;
    }
    if (parsed.kind === "directory") {
      ctxNav?.navigate(directoryRef(parsed.path), { newTab });
      return true;
    }
    if (parsed.kind === "git-commit") {
      ctxNav?.navigate(gitCommitRef(parsed.sha), { newTab });
      return true;
    }
    if (parsed.kind === "internal") {
      ctxNav?.navigate(wikiPageRef(parsed.slug), { newTab });
      return true;
    }
    return false;
  };

  return (
    <div
      className={`oxplow-rt-field ${className ?? ""}`.trim()}
      style={wrapperStyle}
      onClick={(event) => {
        if (handleAnchorIntent(event, false)) return;
        // Clicking anywhere on the wrapper focuses the editor — keeps
        // the "the whole block is editable" feel from Linear.
        if (editor && !editor.isFocused) editor.commands.focus("end");
      }}
      onAuxClick={(event) => {
        // Middle-click on a link → new-tab navigate.
        if (event.button === 1) handleAnchorIntent(event, true);
      }}
      onContextMenu={handleContextMenu}
    >
      {!hidePencil ? (
        <Pencil
          size={12}
          aria-hidden
          className="oxplow-rt-pencil"
          style={{
            position: "absolute",
            top: 6,
            right: 6,
            color: "var(--text-secondary)",
            opacity: 0.35,
            pointerEvents: "none",
            transition: "opacity 120ms ease",
          }}
        />
      ) : null}
      <EditorContent editor={editor} />
      {comments && pendingSel && (
        <NewCommentPopover
          rect={pendingSel.rect}
          onCreate={handleCreateComment}
          onDismiss={() => setPendingSel(null)}
        />
      )}
      {comments && activeComment && activeThread && (
        <CommentPopover
          thread={activeThread}
          author={comments.author ?? "user"}
          anchorRect={activeComment.rect}
          onClose={() => setActiveComment(null)}
        />
      )}
      {commentMenu && (
        <ContextMenu
          items={commentMenu.items}
          position={{ x: commentMenu.x, y: commentMenu.y }}
          onClose={() => setCommentMenu(null)}
        />
      )}
    </div>
  );
}

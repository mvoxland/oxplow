import { useEffect, useRef } from "react";
import type { CSSProperties, MouseEvent as ReactMouseEvent } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown } from "tiptap-markdown";
import { Pencil } from "lucide-react";
import { InternalLink } from "./InternalLink.js";
import { MermaidBlock } from "./MermaidBlock.js";
import { parseMarkdownLink } from "../Wiki/MarkdownView.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef, directoryRef, gitCommitRef, wikiPageRef } from "../../tabs/pageRefs.js";
import { DISK } from "../../file-version.js";

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
}

export function RichTextField({
  value,
  onCommit,
  placeholder,
  inlineOnly = false,
  className,
  style,
  hidePencil,
}: RichTextFieldProps) {
  const lastCommittedRef = useRef(value);
  const debounceRef = useRef<number | null>(null);

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
    </div>
  );
}

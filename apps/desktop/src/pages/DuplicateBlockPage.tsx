import React, { useEffect, useRef, useState } from "react";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { readFile, type Stream } from "../api.js";
import { languageForPath } from "../editor-language.js";
import { shortLabelForVersion, type FileVersion } from "../file-version.js";
import type { DuplicateBlockPayload } from "../tabs/pageRefs.js";

const HIGHLIGHT_STYLE_ID = "oxplow-duplicate-block-style";

function ensureHighlightStyle(): void {
  if (typeof document === "undefined") return;
  if (document.getElementById(HIGHLIGHT_STYLE_ID)) return;
  const style = document.createElement("style");
  style.id = HIGHLIGHT_STYLE_ID;
  style.textContent = `
.monaco-editor .oxplow-duplicate-block-line {
  background: rgba(255, 200, 80, 0.18) !important;
}
.monaco-editor .oxplow-duplicate-block-margin {
  background: rgba(255, 200, 80, 0.35) !important;
}
`;
  document.head.appendChild(style);
}

export interface DuplicateBlockPageProps {
  stream: Stream;
  payload: DuplicateBlockPayload;
  visible: boolean;
  onJumpToSource(path: string, version: FileVersion): void;
}

/**
 * Side-by-side viewer for a single duplicate-block finding. Renders
 * two read-only Monaco editors stacked horizontally, each scrolled so
 * its duplicated range starts at the top of the viewport — making the
 * two ranges line up visually. The duplicated lines on each side are
 * highlighted with a soft accent background.
 *
 * Not a Monaco diff editor: a real text-diff would re-align line-by-
 * line on content, which is exactly what we don't want — duplication
 * findings can come from completely different files where Monaco's
 * diff would line up unrelated lines. We want literal range-aligned
 * side-by-side.
 */
/**
 * Shared scroll-sync bus. Each DuplicateSide registers its editor on
 * mount via `set(side, editor)` and unregisters on unmount. When one
 * side fires `onDidScrollChange`, it pushes its scrollTop to the
 * other; the `lock` flag prevents the mirrored setScrollTop from
 * looping back through the second side's listener.
 */
interface ScrollSyncBus {
  left: any;
  right: any;
  lock: boolean;
}

export function DuplicateBlockPage({ stream, payload, visible, onJumpToSource }: DuplicateBlockPageProps) {
  const leftBase = payload.leftPath.split("/").pop() ?? payload.leftPath;
  const rightBase = payload.rightPath.split("/").pop() ?? payload.rightPath;
  usePageTitle(`${leftBase} ↔ ${rightBase} (duplicate)`);

  void visible;

  const syncRef = useRef<ScrollSyncBus>({ left: null, right: null, lock: false });

  return (
    <Page testId="page-duplicate-block" kind="duplicate-block">
      <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
        <div
          style={{
            padding: "4px 10px",
            borderBottom: "1px solid var(--border-subtle)",
            color: "var(--text-muted)",
            fontSize: 11,
            display: "flex",
            gap: 16,
            alignItems: "center",
          }}
        >
          <span>
            Duplicated block:{" "}
            <strong style={{ color: "var(--text-primary)" }}>
              {payload.leftEnd - payload.leftStart + 1} lines
            </strong>
          </span>
          {(() => {
            const leftLabel = shortLabelForVersion(payload.leftVersion);
            const rightLabel = shortLabelForVersion(payload.rightVersion);
            const combined = leftLabel === rightLabel
              ? `Both at @${leftLabel}`
              : `Left @${leftLabel}, right @${rightLabel}`;
            return (
              <span style={{ color: "var(--text-muted)" }}>{combined}</span>
            );
          })()}
        </div>
        <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
          <DuplicateSide
            stream={stream}
            path={payload.leftPath}
            version={payload.leftVersion}
            startLine={payload.leftStart}
            endLine={payload.leftEnd}
            onJumpToSource={onJumpToSource}
            side="left"
            syncRef={syncRef}
          />
          <div style={{ width: 1, background: "var(--border-subtle)" }} />
          <DuplicateSide
            stream={stream}
            path={payload.rightPath}
            version={payload.rightVersion}
            startLine={payload.rightStart}
            endLine={payload.rightEnd}
            onJumpToSource={onJumpToSource}
            side="right"
            syncRef={syncRef}
          />
        </div>
      </div>
    </Page>
  );
}

interface SideProps {
  stream: Stream;
  path: string;
  version: FileVersion;
  startLine: number;
  endLine: number;
  onJumpToSource(path: string, version: FileVersion): void;
  side: "left" | "right";
  syncRef: React.MutableRefObject<ScrollSyncBus>;
}

function DuplicateSide({
  stream,
  path,
  version,
  startLine,
  endLine,
  onJumpToSource,
  side,
  syncRef,
}: SideProps) {
  const versionLabel = shortLabelForVersion(version);
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const modelRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const [editorReady, setEditorReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ensureHighlightStyle();
    let cancelled = false;
    (async () => {
      const monaco = await import("monaco-editor");
      if (cancelled || !hostRef.current) return;
      monacoRef.current = monaco;
      const editor = monaco.editor.create(hostRef.current, {
        automaticLayout: true,
        readOnly: true,
        theme: "vs-dark",
        minimap: { enabled: false },
        renderLineHighlight: "none",
        scrollBeyondLastLine: false,
      });
      editorRef.current = editor;
      syncRef.current[side] = editor;
      // Mirror scroll position to the peer side. The bus's `lock`
      // flag breaks the feedback loop: when we programmatically push
      // scrollTop to the other editor, that editor's listener fires
      // synchronously, sees the lock, and bails out instead of
      // pushing the value back here.
      editor.onDidScrollChange((e: any) => {
        const bus = syncRef.current;
        if (bus.lock) return;
        const peer = side === "left" ? bus.right : bus.left;
        if (!peer) return;
        bus.lock = true;
        try {
          peer.setScrollTop(e.scrollTop, 1 /* Immediate */);
          peer.setScrollLeft(e.scrollLeft, 1);
        } finally {
          bus.lock = false;
        }
      });
      setEditorReady(true);
    })();
    return () => {
      cancelled = true;
      const editor = editorRef.current;
      const model = modelRef.current;
      editorRef.current = null;
      modelRef.current = null;
      if (syncRef.current[side] === editor) {
        syncRef.current[side] = null;
      }
      editor?.setModel(null);
      editor?.dispose();
      model?.dispose();
    };
  }, [side, syncRef]);

  useEffect(() => {
    if (!editorReady) return;
    let cancelled = false;
    (async () => {
      try {
        const content = await readFile(stream.id, path, version);
        if (cancelled) return;
        const monaco = monacoRef.current;
        const editor = editorRef.current;
        if (!monaco || !editor) return;
        const language = languageForPath(path) ?? "plaintext";
        const model = monaco.editor.createModel(content ?? "", language);
        const previous = modelRef.current;
        editor.setModel(model);
        modelRef.current = model;
        previous?.dispose();

        const lineCount = model.getLineCount();
        const safeStart = Math.max(1, Math.min(startLine, lineCount));
        const safeEnd = Math.max(safeStart, Math.min(endLine, lineCount));
        editor.deltaDecorations(
          [],
          [
            {
              range: new monaco.Range(safeStart, 1, safeEnd, model.getLineMaxColumn(safeEnd)),
              options: {
                isWholeLine: true,
                className: "oxplow-duplicate-block-line",
                marginClassName: "oxplow-duplicate-block-margin",
              },
            },
          ],
        );
        // Pin the duplicate's start line at exactly PAD_LINES from
        // the top of the viewport on both sides. revealLineNearTop()
        // leaves padding to Monaco's discretion, so two sides with
        // different content above the start line land at slightly
        // different scroll offsets — visibly out of sync. Computing
        // a deterministic scrollTop from the line number + a fixed
        // line-height padding makes both editors land identically.
        const PAD_LINES = 2;
        const lineHeight: number = editor.getOption(
          monaco.editor.EditorOption.lineHeight,
        );
        const top: number = editor.getTopForLineNumber(safeStart);
        editor.setScrollTop(Math.max(0, top - lineHeight * PAD_LINES), 1 /* Immediate */);
        editor.setPosition({ lineNumber: safeStart, column: 1 });
        setError(null);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    editorReady,
    stream.id,
    path,
    version.kind,
    version.kind === "ref" ? version.ref : null,
    version.kind === "snapshot" ? version.id : null,
    startLine,
    endLine,
  ]);

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, minWidth: 0 }}>
      <div
        style={{
          padding: "4px 10px",
          borderBottom: "1px solid var(--border-subtle)",
          fontSize: 11,
          display: "flex",
          gap: 8,
          alignItems: "center",
        }}
      >
        <span style={{ fontFamily: "ui-monospace, monospace", color: "var(--text-primary)" }}>
          {path}
        </span>
        <span style={{ color: "var(--text-muted)" }}>
          :{startLine}-{endLine}
        </span>
        <span
          style={{
            fontFamily: "ui-monospace, monospace",
            color: "var(--text-muted)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 3,
            padding: "0 6px",
          }}
          title={`Version: ${versionLabel}`}
        >
          @{versionLabel}
        </span>
        {error ? <span style={{ color: "#ff6b6b" }}>{error}</span> : null}
        <span style={{ flex: 1 }} />
        <button
          type="button"
          onClick={() => onJumpToSource(path, version)}
          style={{
            background: "var(--surface-card)",
            color: "var(--text-primary)",
            borderWidth: 1,
            borderStyle: "solid",
            borderColor: "var(--border-subtle)",
            borderRadius: 3,
            padding: "2px 8px",
            fontSize: 11,
            cursor: "pointer",
          }}
        >
          Open file
        </button>
      </div>
      <div ref={hostRef} style={{ flex: 1, minHeight: 0 }} />
    </div>
  );
}

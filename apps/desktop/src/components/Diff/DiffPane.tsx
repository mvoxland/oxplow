import React, { useEffect, useRef, useState } from "react";
import { readFileAtRef, readWorkspaceFile, type Stream } from "../../api.js";
import { languageForPath } from "../../editor-language.js";

export interface DiffSpec {
  path: string;
  leftRef: string;
  rightKind: "working" | { ref: string };
  baseLabel: string;
  /** When set, skip reading the left side and diff this literal text instead. */
  leftContent?: string;
  /** When set, skip reading the right side and diff this literal text instead. */
  rightContent?: string;
  /** Optional override for the tab label suffix shown next to the filename. */
  labelOverride?: string;
  /** When set, scroll/reveal this 1-based line on the modified (right)
   *  side after both models load, and select it so the position is
   *  obvious. Used by the Change Analysis function rows to land on the
   *  function's start line. */
  revealLine?: number;
}

interface Props {
  stream: Stream;
  spec: DiffSpec;
  visible: boolean;
  /** Open the right-side path in the regular editor pane and close this diff tab. */
  onJumpToSource?: (path: string) => void;
}

const toolbarButtonStyle: React.CSSProperties = {
  background: "var(--panel)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 3,
  padding: "2px 8px",
  fontSize: 11,
  cursor: "pointer",
};

export function DiffPane({ stream, spec, visible, onJumpToSource }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const modelsRef = useRef<{ left: any; right: any } | null>(null);
  const monacoRef = useRef<any>(null);
  const [editorReady, setEditorReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const monaco = await import("monaco-editor");
      if (cancelled || !hostRef.current) return;
      monacoRef.current = monaco;
      const editor = monaco.editor.createDiffEditor(hostRef.current, {
        automaticLayout: true,
        readOnly: true,
        renderSideBySide: true,
        theme: "vs-dark",
        minimap: { enabled: false },
      });
      editorRef.current = editor;
      setEditorReady(true);
    })();
    return () => {
      cancelled = true;
      const editor = editorRef.current;
      const models = modelsRef.current;
      editorRef.current = null;
      modelsRef.current = null;
      editor?.setModel(null);
      editor?.dispose();
      models?.left?.dispose();
      models?.right?.dispose();
    };
  }, []);

  useEffect(() => {
    if (!stream || !editorReady) return;
    let cancelled = false;
    (async () => {
      try {
        const leftPromise = spec.leftContent !== undefined
          ? Promise.resolve({ content: spec.leftContent })
          : readFileAtRef(stream.id, spec.leftRef, spec.path);
        const rightPromise = spec.rightContent !== undefined
          ? Promise.resolve({ content: spec.rightContent })
          : spec.rightKind === "working"
            ? readWorkspaceFile(stream.id, spec.path).then(
                (file) => ({ content: file.content as string | null }),
                () => ({ content: null as string | null }),
              )
            : readFileAtRef(stream.id, spec.rightKind.ref, spec.path);
        const [leftResult, rightResult] = await Promise.all([leftPromise, rightPromise]);
        if (cancelled) return;
        const monaco = monacoRef.current;
        const editor = editorRef.current;
        if (!monaco || !editor) return;
        const language = languageForPath(spec.path) ?? "plaintext";
        const left = monaco.editor.createModel(leftResult.content ?? "", language);
        const right = monaco.editor.createModel(rightResult.content ?? "", language);
        const previous = modelsRef.current;
        editor.setModel({ original: left, modified: right });
        modelsRef.current = { left, right };
        previous?.left?.dispose();
        previous?.right?.dispose();
        setError(null);
        if (spec.revealLine && spec.revealLine > 0) {
          // Reveal on the modified (right) editor, which is what the
          // analysis-page jump targets. The diff editor exposes the
          // sub-editor via getModifiedEditor(); fall back gracefully
          // if Monaco's API ever shifts shape.
          const modifiedEditor = (editor as { getModifiedEditor?: () => any }).getModifiedEditor?.();
          if (modifiedEditor) {
            const line = spec.revealLine;
            modifiedEditor.revealLineInCenter(line);
            modifiedEditor.setPosition({ lineNumber: line, column: 1 });
            modifiedEditor.setSelection({ startLineNumber: line, startColumn: 1, endLineNumber: line, endColumn: 1 });
            modifiedEditor.focus();
          }
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => { cancelled = true; };
  }, [stream, editorReady, spec.path, spec.leftRef, typeof spec.rightKind === "string" ? "working" : spec.rightKind.ref, spec.leftContent, spec.rightContent, spec.revealLine]);

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
      <div style={{ padding: "4px 10px", borderBottom: "1px solid var(--border)", color: "var(--muted)", fontSize: 11, display: "flex", gap: 8, alignItems: "center" }}>
        <span style={{ fontFamily: "ui-monospace, monospace", color: "var(--fg)" }}>{spec.path}</span>
        <span>vs {spec.baseLabel}</span>
        {error ? <span style={{ color: "#ff6b6b" }}>{error}</span> : null}
        <span style={{ flex: 1 }} />
        <button
          type="button"
          onClick={() => editorRef.current?.goToDiff("previous")}
          disabled={!editorReady}
          title="Previous change"
          data-testid="diff-prev-change"
          style={toolbarButtonStyle}
        >
          ↑ Prev
        </button>
        <button
          type="button"
          onClick={() => editorRef.current?.goToDiff("next")}
          disabled={!editorReady}
          title="Next change"
          data-testid="diff-next-change"
          style={toolbarButtonStyle}
        >
          ↓ Next
        </button>
        <button
          type="button"
          onClick={() => onJumpToSource?.(spec.path)}
          disabled={!onJumpToSource}
          title="Open file in editor"
          data-testid="diff-jump-to-source"
          style={toolbarButtonStyle}
        >
          Open file
        </button>
      </div>
      <div
        ref={hostRef}
        style={{
          flex: 1,
          minHeight: 0,
          display: visible ? "block" : "none",
        }}
      />
    </div>
  );
}

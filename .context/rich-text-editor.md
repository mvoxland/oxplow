# Rich-text editor (shared WYSIWYG surface)

What this doc covers: the Tiptap-based `RichTextField` editor used for
inline-editable prose on task and (future) wiki pages, how mermaid +
internal URL schemes survive round-trip, and the composition model
("one page, many editors, mixed editability") it was built for.

## Composition model

Pages that read as a single document are composed at the React level
from multiple independent blocks:

- **Editable blocks** — one `RichTextField` per field. On the task
  page, title → `task.title` (via the simpler `TitleField` textarea)
  and description → `task.description`. Each block runs its own
  `onCommit` and saves to its own destination.
- **Read-only blocks** — rendered with plain React + `MarkdownView`,
  no editor instance. Effort cards in `ActivityTimeline` are the
  current example; future "referenced files" footers on wiki pages
  follow the same pattern.

The visual continuity ("it feels like one document") comes from
shared prose typography (`.oxplow-md` styles applied to both the
editor surface and `MarkdownView`), not from a single underlying
editor.

To telegraph editability, every editable block carries a faint
`Pencil` icon (lucide-react, ~12px) in its top-right, opacity ~0.35
by default and ~0.85 on hover/focus. Read-only blocks must **not**
show the pencil — that's the consistent signal "this is for reading."

## What's in `apps/desktop/src/components/RichText/`

- **`RichTextField.tsx`** — the surface component. Configures Tiptap
  with `StarterKit` + `Markdown` (round-trip) + `Placeholder` +
  `MermaidBlock` + `InternalLink`. Debounced 300ms save while typing,
  immediate commit on blur. The editor's outer `<div>` wears
  `.oxplow-rt-field` (hover tint + pencil reveal) and the inner
  ProseMirror element wears `.oxplow-md .oxplow-rt-editor` so prose
  inherits the same typography as `MarkdownView`.
- **`MermaidBlock.tsx`** — extends Tiptap's `CodeBlock` (same node
  name, `codeBlock`) with a React NodeView that paints rendered SVG
  via `renderMermaidInto` when the caret is outside, and a raw
  editable `<pre><code>` when the caret enters. Round-trips as a
  ` ```mermaid …``` ` fenced code block, so storage is unchanged.
- **`InternalLink.ts`** — extends Tiptap's standard `Link` mark to
  allow `file:`, `dir:`, `gitcommit:` URL schemes through the URL
  sanitizer. `openOnClick: false` — click handling is owned by the
  page-level handler (currently the read view; in-editor click
  routing through `useOptionalPageNavigation` is a follow-up).

## What's in `apps/desktop/src/components/Wiki/mermaidRender.ts`

Shared mermaid rendering pipeline used by `MermaidBlock` (editor
NodeView). `MarkdownView` still has its own inline copy of the same
logic; consolidating them is a small follow-up. Exports
`loadMermaid`, `loadSvgPanZoom`, `attachPanZoom`,
`renderMermaidInto`. Lazy-loads mermaid + svg-pan-zoom on first use;
waits for the host element to have a non-zero layout box before
initializing pan-zoom (otherwise `getCTM().inverse()` throws on
zero-size SVGs — see `MarkdownView.tsx` for the original guard).

## Storage model

Markdown stays the on-disk format. `tiptap-markdown` parses on mount
and serializes on save via `editor.storage.markdown.getMarkdown()`.
Fenced code blocks round-trip; lists, headings, bold, italic, code,
blockquotes, GFM tables — all standard. The custom `MermaidBlock`
hijacks rendering of `language: "mermaid"` fences without changing
the serialized form.

## Wikilink round-trip

Wiki pages use `[[ ]]` syntax extensively (`[[path/to/file]]`,
`[[dir:src/components|the folder]]`, `[[git:<sha>]]`, etc.).
`MarkdownView`'s `preprocessWikilinks` converts these to standard
markdown links (`[label](file:path)`) for read-rendering; the new
`postprocessWikilinks` helper is the inverse — it collapses standard
markdown links carrying our internal schemes (`file:`, `dir:`,
`gitcommit:`) back into `[[ ]]` form on save.

`WikiPageTab` applies `preprocessWikilinks` to the body before
handing it to `RichTextField`, and applies `postprocessWikilinks` to
the markdown it gets back on `onCommit` before calling
`writeWikiPageBody`. The on-disk shape is preserved across edits,
including the bare vs. labeled forms (`[[path]]` vs.
`[[path|label]]`). Plain http/https links and image links are left
alone — collapsing those into `[[ ]]` form would be lossy.

## What does NOT round-trip yet

- **Internal-link click routing.** Inside the editor, clicking a
  `file:`/`dir:`/`gitcommit:` link does nothing — `openOnClick:
  false`. The read-only `MarkdownView` path still routes via
  `useOptionalPageNavigation`. Adding in-editor click routing is a
  follow-up.
- **Per-link kebab menus / wiki title resolution.** `MarkdownView`
  attaches a hover-revealed kebab to every link (copy URL, open in
  new tab, etc.) and swaps bare `[[slug]]` text for the resolved
  wiki page title via `useWikiTitle`. Neither is implemented in the
  editor surface yet.

## CSS surfaces

In `apps/desktop/index.html`:

- `.oxplow-md` — shared prose typography. Applied to both
  `MarkdownView` wrappers and the editor's inner ProseMirror element.
- `.oxplow-rt-field` — outer wrapper. Hover/focus add a
  `--surface-card` background + `--border-strong` outline; this is
  where the pencil-affordance reveal triggers from.
- `.oxplow-rt-editor.ProseMirror` — kills the default focus outline,
  sets `min-height: 1.4em`, and implements the placeholder
  pseudo-element (Tiptap's `Placeholder` extension drives the
  `data-placeholder` attribute).
- `.task-section-heading`, `.task-title-field` — task page specifics.

## Where the editor is used today

- `apps/desktop/src/components/Plan/TaskDetail.tsx` — description +
  acceptance fields on the task details page. Title uses the simpler
  `TitleField` (auto-sizing textarea, no rich formatting).
- `apps/desktop/src/components/Wiki/WikiPageTab.tsx` — wiki page body.
  Always-on editing (no view/edit toggle); the Edit / Save / Revert /
  Done-editing buttons were removed from `WikiPageRail`. Save fires
  via the RichTextField debounce + blur, calling `writeWikiPageBody`
  directly (with `postprocessWikilinks` first). Errors surface via
  `recordOpError`. The `useWikiPageController` hook still backs
  page-summary loading, refresh-on-event, and notFound/loadError
  state — it just no longer owns the editor's draft.

## Related

- [theming.md](./theming.md) — semantic CSS variable tokens.
- [usability.md](./usability.md) — Enter/Escape contracts inline-edit
  fields must follow. RichTextField uses Cmd/Ctrl+Enter for explicit
  commit (Enter inserts a paragraph) + blur for implicit commit.

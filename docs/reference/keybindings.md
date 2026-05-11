# Keybindings

Oxplow follows two ironclad UI rules everywhere they make sense:

- **Enter submits.** Whatever input you're in, Enter is the
  affirmative action.
- **Escape cancels.** Whatever input you're in, Escape gets you
  out without applying changes.

Beyond those, the keymap is intentionally small — the product
has a directory of pages, not a thousand commands. The
authoritative list lives in
`apps/desktop/src/keybindings.ts`.

## Global

| Action | macOS | Windows / Linux |
|---|---|---|
| Save active file | `Cmd+S` | `Ctrl+S` |
| Quick open (file + page picker) | `Cmd+P` | `Ctrl+P` |
| Find in current editor | `Cmd+F` | `Ctrl+F` |
| File new task (in plan view) | `Cmd+Shift+N` | `Ctrl+Shift+N` |

`Cmd+P` opens the QuickOpen overlay — fuzzy-match across files
in the active stream's worktree plus oxplow's named pages (Plan
work, Backlog, Local History, Change Analysis, Git dashboard,
etc.).

## Tabs and pages

| Action | Key |
|---|---|
| Open link in new tab | `Cmd/Ctrl+Click`, middle-click, or right-click on any `RouteLink` |
| Browser back / forward | Click the back / forward buttons in the page nav bar |
| Bookmark current page | Click the star in the page nav bar |
| Close active tab | Tab kebab → Close |

Browser-tab semantics are non-negotiable: plain-click navigates
*in-tab*; only Cmd/Ctrl-click + middle-click + right-click open a
new tab. The rule is enforced through the single `RouteLink` /
`useRouteDispatch` chokepoint, so list views and tree rows behave
the same way as link buttons.

## Editor (Monaco)

Standard Monaco bindings apply. The most-used:

| Action | macOS | Windows / Linux |
|---|---|---|
| Save | `Cmd+S` | `Ctrl+S` |
| Find / replace | `Cmd+F` / `Cmd+Alt+F` | `Ctrl+F` / `Ctrl+H` |
| Multi-cursor | `Cmd+Click` | `Ctrl+Click` |
| Go to definition | `F12` | `F12` |
| Go to line | `Cmd+G` | `Ctrl+G` |
| Toggle line comment | `Cmd+/` | `Ctrl+/` |

LSP go-to-definition / hover / references all flow through the
oxplow LSP bridge — same servers the agent calls via MCP.

## tasks (Plan / Backlog / Done / Archived pages)

| Action | Key |
|---|---|
| Open inline new-row | `Cmd/Ctrl+Shift+N` (or click the row at the top of the list) |
| File the new row | type a title and press `Enter` |
| Edit selected item | `Enter` |
| Cancel inline edit | `Escape` |
| Delete / archive / cancel | row kebab — no single-key shortcut |
| Reorder | drag with the row handle |

Destructive actions (delete, archive, cancel) are deliberately
behind a kebab popover, never a single keystroke — keeps the
queue from losing data to a stray keypress. The whole product
uses kebabs instead of right-click menus for the same reason:
discoverability over chord memorization.

## Terminal

| Action | macOS | Windows / Linux |
|---|---|---|
| Paste | `Cmd+V` | `Ctrl+Shift+V` (xterm-default) |
| Copy | selection → release (auto-copy) | selection → release |
| Per-tab Copy / Paste / Clear | header kebab (replaces xterm right-click) |

## Drop targets

When you drag a file, task, or rail entry, the target zone
highlights as you drag. Drop highlighting is *the* signal that a
drop will work — if you don't see a highlight, the drop isn't
supported there.

The agent terminal accepts drag-to-add-context from task
rows, file rows, the rail's recent-files / active item / up-next
lists, backlinks entries, and code-quality file groups.

## Why so few

VS Code has hundreds of keybindings. Oxplow has a handful on
purpose. The product is small enough to learn the rail and the
agent does most of the typing — keybindings are for navigation
and submit / cancel, not for the long tail of editor commands.

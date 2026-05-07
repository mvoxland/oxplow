---
date: 2026-05-07
categories:
  - Releases
---

# Oxplow 0.3 -- Rewriting the foundation

0.3 is, more than anything else, a rewrite. Switched from Electron to Tauri, same Typescript on top but now with Rust underneath. Functionally, added a Change Analysis dashboard that treats "what did this diff do" as a first-class question, and a steady push to make the whole app feel like a browser instead of a desktop app full of modals.

<!-- more -->

## Change Analysis is a thing now

The Change Analysis page started life as a side panel for diff stats. In 0.3 it grew into a real dashboard -- the place you go to *understand* a change, not just look at a unified diff.

Open it against any pair of refs and you get:

- **A scoped summary** with a *Look here first* card that ranks files by a CRAP-flavored interestingness score (churn × complexity × tests-missing × duplication, multiplicative so a single hot factor dominates). Tuneable from one place.
- **Pivots that drill down.** Click a file extension, directory, or status (added / modified / deleted) and the dashboard re-renders as a focused drilldown over just that slice -- same hook, scope applied. The drilldown carries a Semantic / File-list view toggle, a status filter, and relocated duplication + tests cards.
- **Per-function before/after metrics.** A new `analyze_functions_at_refs` IPC walks both sides of the diff and buckets functions into added / deleted / signature-changed / body-changed, with cyclomatic complexity at each end. The Function Churn card uses a per-function variant of the same interestingness score for tiebreak ordering.
- **Sibling-aware diff opens.** Clicking through to the actual unified diff brings sibling page lists and a jump-to dropdown, so reviewing a 40-file change isn't a back-button pilgrimage.
- **Code quality scanners moved in-process.** No more shelling out to lizard / jscpd -- complexity and duplication run as Rust scanners against tree-sitter, with findings persisted into the same SQLite store everything else uses. The Change Analysis cards read from those findings directly.

The pitch is small but specific: when you sit down to review a branch, the first screen shouldn't be a flat file list. It should tell you which three files to look at first and why.

## Pushing harder on the "web" style UI

0.2 was already a tabbed app. 0.3 leans into the rest of what makes a browser feel like a browser:

- **Browser-tab click semantics, everywhere.** Plain-click navigates in-tab; Cmd/Ctrl-click, middle-click, and right-click open a new tab. Every clickable row that targets another tab now routes through a single `RouteLink` / `useRouteDispatch` chokepoint, so the rule doesn't quietly regress when a new list view shows up.
- **A real nav bar on every page.** Back / forward, bookmark, backlinks dropdown -- mounted by the shared `Page` chrome and powered by a `PageNavigationContext` that descendants can drive. Wiki-link clicks inside a note participate in tab-level history. Pages register their title once and the same string drives the chrome header *and* the tab strip label, so there's no per-page duplicate header markup anymore.
- **Bookmarks + a left-rail HUD.** Per-scope (thread / stream / global) bookmarks, surfaced in a persistent rail with single-letter scope badges next to recent files and the pages directory. The rail is passive -- it never auto-opens tabs -- which keeps the focus where you put it.
- **A center-tabs overflow dropdown.** LRU eviction at 20 tabs, plus the dropdown for everything that doesn't fit, so heavy navigation sessions stay legible.
- **External URLs open in a sandboxed Tauri window** that's isolated from the main webview, so following a link out of a note or backlink doesn't poke a hole in the renderer.

## Everything else

A long tail of smaller things came along for the ride:

- **Git is more honest about state.** Branch reconciliation against live HEAD on refresh and seed. Recent commits surface branch + tag labels next to each row. Capped diff navigation, sibling page lists with the jump-to dropdown.
- **Editor and terminal pulled their weight.** Clickable file-path links in the terminal pane. Better blame overlay. The Monaco bridge knows about the LSP layer.
- **LSP installer.** Fetches Mason packages for the supported languages, caches them under `.oxplow/lsp/`, and the proxy hands the right binary to whichever stream asked. Clojure joined the supported list as the tenth language.
- **Live agent turns surface in the work panel.** Each open turn renders as a passive in-progress row with the prompt and a spinner; no synthesized work items, no narration.

## Where it stands

Still very alpha and under construction. But it's starting to work well for developing itself in.

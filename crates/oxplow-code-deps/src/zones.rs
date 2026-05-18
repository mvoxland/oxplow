//! Architectural zone classification for repo files.
//!
//! Every file in the project is assigned to exactly one [`Zone`]
//! based on its repo-relative path. The mapping is intentionally
//! coarse — fewer than 20 zones — so the resulting "zone bar" stays
//! legible. Files that match no specific prefix fall through to
//! [`Zone::Other`].
//!
//! The table is hard-coded here rather than read from disk to keep
//! classification cheap (no I/O, no parsing) and reproducible. When
//! a new top-level area appears in the repo, add an entry below.
//!
//! ## How rules are applied
//!
//! Rules are evaluated in declaration order and the FIRST match
//! wins. The table is therefore sorted most-specific-first: e.g.
//! `apps/desktop/src-tauri/` (shell) before `apps/desktop/` (ui).
//! Each rule is a simple `starts_with` prefix or a glob-style
//! suffix; see [`ZoneRule`] for the variants.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ImportEdge;

/// Architectural zone. Coarse on purpose — one chip in the UI per
/// distinct concept. Add new variants here when a new top-level
/// concern appears in the repo.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    /// Desktop frontend React code (`apps/desktop/src/**` outside
    /// `src-tauri`).
    Ui,
    /// Tauri shell crate (`apps/desktop/src-tauri/`).
    Shell,
    /// `#[tauri::command]` adapters that bridge UI ↔ services.
    Ipc,
    /// Pure-types + store traits (`oxplow-domain`).
    Domain,
    /// rusqlite stores + migrations (`oxplow-db`).
    Store,
    /// Git integration (`oxplow-git`).
    Git,
    /// LSP bridge crates (`oxplow-lsp`, `oxplow-lsp-installer`).
    Lsp,
    /// Runtime / write-guard / filing enforcement (`oxplow-runtime`).
    Runtime,
    /// Filesystem watchers (`oxplow-fs-watch`).
    FsWatch,
    /// PTY + tmux subsystems.
    Terminal,
    /// MCP server (`oxplow-mcp`).
    Mcp,
    /// Top-level Services orchestration (`oxplow-app`).
    AppOrchestration,
    /// Config crate.
    Config,
    /// Session crate.
    Session,
    /// Plugin / control-plane.
    Plugin,
    /// Static analysis crates (`oxplow-code-metrics`,
    /// `oxplow-code-dup`, `oxplow-code-deps`, `oxplow-tree-source`).
    Analysis,
    /// Database migrations specifically (across crates).
    Migration,
    /// Test files (`*_test.rs`, `*.test.ts`, `tests/` directories).
    Test,
    /// Documentation, README, `.context/`, ADRs.
    Docs,
    /// Project metadata (Cargo.toml, package.json, tauri.conf.json,
    /// .toml/.json config at the repo root).
    ProjectMeta,
    /// Anything outside the repo (external crate / npm package /
    /// system header) — only used for import targets, never for
    /// files in the worktree.
    External,
    /// Anything else.
    Other,
}

impl Zone {
    /// Stable short label for UI chips. Kept ≤ 8 chars where
    /// possible so a horizontal zone bar stays compact.
    pub fn short_label(self) -> &'static str {
        match self {
            Zone::Ui => "ui",
            Zone::Shell => "shell",
            Zone::Ipc => "ipc",
            Zone::Domain => "domain",
            Zone::Store => "store",
            Zone::Git => "git",
            Zone::Lsp => "lsp",
            Zone::Runtime => "runtime",
            Zone::FsWatch => "fs-watch",
            Zone::Terminal => "terminal",
            Zone::Mcp => "mcp",
            Zone::AppOrchestration => "app",
            Zone::Config => "config",
            Zone::Session => "session",
            Zone::Plugin => "plugin",
            Zone::Analysis => "analysis",
            Zone::Migration => "migration",
            Zone::Test => "test",
            Zone::Docs => "docs",
            Zone::ProjectMeta => "meta",
            Zone::External => "external",
            Zone::Other => "other",
        }
    }
}

/// Internal rule shape. Each rule is checked in order; the first
/// matching one wins.
enum ZoneRule {
    /// Match when the (normalized) path starts with this prefix.
    Prefix(&'static str, Zone),
    /// Match when the path contains this segment (e.g.
    /// `/migrations/` regardless of which crate owns it).
    Contains(&'static str, Zone),
    /// Match when the basename ends with this suffix (e.g.
    /// `_test.rs`, `.test.ts`).
    Suffix(&'static str, Zone),
    /// Match when the basename is exactly this string (for
    /// well-known root-level files).
    Basename(&'static str, Zone),
}

/// The rule table. Order matters — most-specific patterns first.
static RULES: &[ZoneRule] = &[
    // ---- Project metadata first — these basenames identify a
    // file by its role regardless of which crate or app owns it.
    ZoneRule::Basename("Cargo.toml", Zone::ProjectMeta),
    ZoneRule::Basename("Cargo.lock", Zone::ProjectMeta),
    ZoneRule::Basename("package.json", Zone::ProjectMeta),
    ZoneRule::Basename("bun.lockb", Zone::ProjectMeta),
    ZoneRule::Basename("tauri.conf.json", Zone::ProjectMeta),
    ZoneRule::Basename("tsconfig.json", Zone::ProjectMeta),
    // ---- Test files (before crate-based zones so a test file
    // inside oxplow-db is classified as Test, not Store).
    ZoneRule::Suffix("_test.rs", Zone::Test),
    ZoneRule::Suffix(".test.ts", Zone::Test),
    ZoneRule::Suffix(".test.tsx", Zone::Test),
    ZoneRule::Suffix(".spec.ts", Zone::Test),
    ZoneRule::Suffix(".spec.tsx", Zone::Test),
    ZoneRule::Suffix("_test.go", Zone::Test),
    ZoneRule::Contains("/tests/", Zone::Test),
    ZoneRule::Contains("/__tests__/", Zone::Test),
    // ---- Migrations (any crate).
    ZoneRule::Contains("/migrations/", Zone::Migration),
    // ---- Docs.
    ZoneRule::Prefix(".context/", Zone::Docs),
    ZoneRule::Suffix(".md", Zone::Docs),
    ZoneRule::Basename("README", Zone::Docs),
    // ---- Desktop frontend / shell.
    ZoneRule::Prefix("apps/desktop/src-tauri/", Zone::Shell),
    ZoneRule::Prefix("apps/desktop/src/", Zone::Ui),
    ZoneRule::Prefix("apps/desktop/", Zone::Ui),
    // ---- Rust crates.
    ZoneRule::Prefix("crates/oxplow-tauri-ipc/", Zone::Ipc),
    ZoneRule::Prefix("crates/oxplow-domain/", Zone::Domain),
    ZoneRule::Prefix("crates/oxplow-db/", Zone::Store),
    ZoneRule::Prefix("crates/oxplow-git/", Zone::Git),
    ZoneRule::Prefix("crates/oxplow-lsp-installer/", Zone::Lsp),
    ZoneRule::Prefix("crates/oxplow-lsp/", Zone::Lsp),
    ZoneRule::Prefix("crates/oxplow-runtime/", Zone::Runtime),
    ZoneRule::Prefix("crates/oxplow-fs-watch/", Zone::FsWatch),
    ZoneRule::Prefix("crates/oxplow-tmux/", Zone::Terminal),
    ZoneRule::Prefix("crates/oxplow-pty/", Zone::Terminal),
    ZoneRule::Prefix("crates/oxplow-mcp/", Zone::Mcp),
    ZoneRule::Prefix("crates/oxplow-app/", Zone::AppOrchestration),
    ZoneRule::Prefix("crates/oxplow-config/", Zone::Config),
    ZoneRule::Prefix("crates/oxplow-session/", Zone::Session),
    ZoneRule::Prefix("crates/oxplow-plugin/", Zone::Plugin),
    ZoneRule::Prefix("crates/oxplow-control-plane/", Zone::Plugin),
    ZoneRule::Prefix("crates/oxplow-code-metrics/", Zone::Analysis),
    ZoneRule::Prefix("crates/oxplow-code-dup/", Zone::Analysis),
    ZoneRule::Prefix("crates/oxplow-code-deps/", Zone::Analysis),
    ZoneRule::Prefix("crates/oxplow-tree-source/", Zone::Analysis),
    // ---- Catch-all project metadata for `.toml` files outside the
    // recognized crate prefixes (configs in the repo root, etc.).
    ZoneRule::Suffix(".toml", Zone::ProjectMeta),
];

/// Classify a repo-relative path to its architectural zone.
///
/// Path separators are normalized to `/` before matching, so
/// callers may pass either flavor.
pub fn classify_zone(path: &str) -> Zone {
    let normalized = path.replace('\\', "/");
    let basename = std::path::Path::new(&normalized)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    for rule in RULES {
        let hit = match rule {
            ZoneRule::Prefix(p, _) => normalized.starts_with(p),
            ZoneRule::Contains(p, _) => normalized.contains(p),
            ZoneRule::Suffix(p, _) => basename.ends_with(p),
            ZoneRule::Basename(p, _) => basename == *p,
        };
        if hit {
            return match rule {
                ZoneRule::Prefix(_, z)
                | ZoneRule::Contains(_, z)
                | ZoneRule::Suffix(_, z)
                | ZoneRule::Basename(_, z) => *z,
            };
        }
    }
    Zone::Other
}

/// Map a workspace crate name (e.g. `oxplow-db`) to its zone. Used
/// when classifying Rust import targets that look like
/// `oxplow_db::store::…` — the first path segment maps to a crate.
pub fn zone_for_crate_name(name: &str) -> Option<Zone> {
    // Convention: crates are named `oxplow-foo`, Rust paths use
    // `oxplow_foo`. Normalize to dashes before lookup.
    let dashed = name.replace('_', "-");
    let synthetic_path = format!("crates/{dashed}/src/lib.rs");
    let zone = classify_zone(&synthetic_path);
    if zone == Zone::Other {
        None
    } else {
        Some(zone)
    }
}

/// A directed edge between two zones, with the originating
/// [`ImportEdge`] for hover/drill-down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ZonedImportEdge {
    pub edge: ImportEdge,
    pub from_zone: Zone,
    /// The target zone if we could classify it, else None. None
    /// indicates a target we couldn't resolve (external package,
    /// path the resolver doesn't know how to walk).
    pub to_zone: Option<Zone>,
}

impl ZonedImportEdge {
    /// True when this edge crosses an *internal* architectural
    /// boundary worth flagging. Specifically:
    ///
    /// - both zones must be known,
    /// - the target zone must not be [`Zone::External`] (reaching
    ///   into a third-party crate / npm package isn't a layer
    ///   violation in our architecture),
    /// - `from_zone` and `to_zone` must be distinct.
    ///
    /// Unknown targets (None) never trip cross-zone — better to
    /// underflag than overflag.
    pub fn is_cross_zone(&self) -> bool {
        match self.to_zone {
            None => false,
            Some(Zone::External) => false,
            Some(target) => target != self.from_zone,
        }
    }
}

/// Classify an [`ImportEdge`] with a known target file path (caller-
/// resolved). Both ends are converted via [`classify_zone`].
pub fn zone_for_resolved_edge(edge: ImportEdge, resolved_target: &str) -> ZonedImportEdge {
    let from_zone = classify_zone(&edge.from_path);
    let to_zone = Some(classify_zone(resolved_target));
    ZonedImportEdge {
        edge,
        from_zone,
        to_zone,
    }
}

/// Classify an [`ImportEdge`] whose target couldn't be resolved to a
/// repo file. The `to_zone` is None (we don't pretend to know).
pub fn zone_for_unresolved_edge(edge: ImportEdge) -> ZonedImportEdge {
    let from_zone = classify_zone(&edge.from_path);
    ZonedImportEdge {
        edge,
        from_zone,
        to_zone: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_paths_classify_as_ui() {
        assert_eq!(
            classify_zone("apps/desktop/src/components/Foo.tsx"),
            Zone::Ui
        );
        assert_eq!(classify_zone("apps/desktop/src/stores/tabs.ts"), Zone::Ui);
    }

    #[test]
    fn shell_takes_priority_over_ui_inside_desktop() {
        assert_eq!(
            classify_zone("apps/desktop/src-tauri/src/main.rs"),
            Zone::Shell
        );
    }

    #[test]
    fn crates_map_to_their_zones() {
        assert_eq!(classify_zone("crates/oxplow-db/src/lib.rs"), Zone::Store);
        assert_eq!(classify_zone("crates/oxplow-git/src/blame.rs"), Zone::Git);
        assert_eq!(
            classify_zone("crates/oxplow-tauri-ipc/src/lib.rs"),
            Zone::Ipc
        );
        assert_eq!(
            classify_zone("crates/oxplow-domain/src/work.rs"),
            Zone::Domain
        );
        assert_eq!(
            classify_zone("crates/oxplow-runtime/src/lib.rs"),
            Zone::Runtime
        );
        assert_eq!(classify_zone("crates/oxplow-lsp/src/lib.rs"), Zone::Lsp);
        assert_eq!(
            classify_zone("crates/oxplow-lsp-installer/src/lib.rs"),
            Zone::Lsp
        );
        assert_eq!(
            classify_zone("crates/oxplow-code-deps/src/zones.rs"),
            Zone::Analysis
        );
    }

    #[test]
    fn tests_beat_crate_zone() {
        // A test file inside oxplow-db should be Test, not Store.
        assert_eq!(
            classify_zone("crates/oxplow-db/tests/integration.rs"),
            Zone::Test
        );
        assert_eq!(
            classify_zone("apps/desktop/src/components/Foo.test.tsx"),
            Zone::Test
        );
    }

    #[test]
    fn migrations_classify_uniformly() {
        assert_eq!(
            classify_zone("crates/oxplow-db/migrations/V001__init.sql"),
            Zone::Migration
        );
    }

    #[test]
    fn docs_classify_to_docs() {
        assert_eq!(classify_zone(".context/architecture.md"), Zone::Docs);
        assert_eq!(classify_zone("README.md"), Zone::Docs);
        assert_eq!(classify_zone("README"), Zone::Docs);
    }

    #[test]
    fn project_meta_basenames() {
        assert_eq!(classify_zone("Cargo.toml"), Zone::ProjectMeta);
        assert_eq!(
            classify_zone("apps/desktop/package.json"),
            Zone::ProjectMeta
        );
        assert_eq!(
            classify_zone("apps/desktop/src-tauri/tauri.conf.json"),
            Zone::ProjectMeta
        );
    }

    #[test]
    fn unknown_paths_fall_to_other() {
        assert_eq!(classify_zone("scripts/build.sh"), Zone::Other);
        assert_eq!(classify_zone("misc/random.txt"), Zone::Other);
    }

    #[test]
    fn windows_separators_normalize() {
        assert_eq!(classify_zone("apps\\desktop\\src\\App.tsx"), Zone::Ui);
    }

    #[test]
    fn zone_for_crate_name_handles_underscore_and_dash() {
        assert_eq!(zone_for_crate_name("oxplow-db"), Some(Zone::Store));
        assert_eq!(zone_for_crate_name("oxplow_db"), Some(Zone::Store));
        assert_eq!(zone_for_crate_name("serde"), None);
    }

    #[test]
    fn cross_zone_detects_ui_to_store() {
        let edge = ImportEdge {
            from_path: "apps/desktop/src/components/Foo.tsx".into(),
            raw: "import { db } from '@/store';".into(),
            module: "@/store".into(),
            kind: crate::ImportKind::Import,
            start_line: 1,
            end_line: 1,
        };
        let zoned = zone_for_resolved_edge(edge, "crates/oxplow-db/src/lib.rs");
        assert_eq!(zoned.from_zone, Zone::Ui);
        assert_eq!(zoned.to_zone, Some(Zone::Store));
        assert!(zoned.is_cross_zone());
    }

    #[test]
    fn same_zone_not_cross_zone() {
        let edge = ImportEdge {
            from_path: "crates/oxplow-db/src/lib.rs".into(),
            raw: "use crate::migrations;".into(),
            module: "crate::migrations".into(),
            kind: crate::ImportKind::Use,
            start_line: 1,
            end_line: 1,
        };
        let zoned = zone_for_resolved_edge(edge, "crates/oxplow-db/src/migrations.rs");
        assert!(!zoned.is_cross_zone());
    }

    #[test]
    fn unresolved_edge_is_not_cross_zone() {
        // External crate (std, serde, etc.) — to_zone is None and we
        // should NOT flag the edge as cross-zone.
        let edge = ImportEdge {
            from_path: "crates/oxplow-db/src/lib.rs".into(),
            raw: "use serde::Serialize;".into(),
            module: "serde::Serialize".into(),
            kind: crate::ImportKind::Use,
            start_line: 1,
            end_line: 1,
        };
        let zoned = zone_for_unresolved_edge(edge);
        assert_eq!(zoned.to_zone, None);
        assert!(!zoned.is_cross_zone());
    }
}

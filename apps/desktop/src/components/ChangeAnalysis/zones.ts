// Path → architectural zone classifier. Mirrors the table in
// `crates/oxplow-code-deps/src/zones.rs`. Kept in sync by hand —
// when you change one, change the other. The Rust crate is the
// source of truth for analysis output (its serialized `Zone` lands
// on `ZonedImportEdge` records); this TS copy lets the UI badge
// files in the file tree without a backend roundtrip.

export type Zone =
  | "ui"
  | "shell"
  | "ipc"
  | "domain"
  | "store"
  | "git"
  | "lsp"
  | "runtime"
  | "fs_watch"
  | "terminal"
  | "mcp"
  | "app_orchestration"
  | "config"
  | "session"
  | "plugin"
  | "analysis"
  | "migration"
  | "test"
  | "docs"
  | "project_meta"
  | "external"
  | "other";

export const ZONE_LABELS: Record<Zone, string> = {
  ui: "ui",
  shell: "shell",
  ipc: "ipc",
  domain: "domain",
  store: "store",
  git: "git",
  lsp: "lsp",
  runtime: "runtime",
  fs_watch: "fs-watch",
  terminal: "terminal",
  mcp: "mcp",
  app_orchestration: "app",
  config: "config",
  session: "session",
  plugin: "plugin",
  analysis: "analysis",
  migration: "migration",
  test: "test",
  docs: "docs",
  project_meta: "meta",
  external: "external",
  other: "other",
};

type Rule =
  | { type: "basename"; value: string; zone: Zone }
  | { type: "prefix"; value: string; zone: Zone }
  | { type: "contains"; value: string; zone: Zone }
  | { type: "suffix"; value: string; zone: Zone };

// Order matters — first match wins. Keep aligned with RULES in
// crates/oxplow-code-deps/src/zones.rs.
const RULES: Rule[] = [
  // Project metadata first — basenames identify role regardless of
  // crate ownership.
  { type: "basename", value: "Cargo.toml", zone: "project_meta" },
  { type: "basename", value: "Cargo.lock", zone: "project_meta" },
  { type: "basename", value: "package.json", zone: "project_meta" },
  { type: "basename", value: "bun.lockb", zone: "project_meta" },
  { type: "basename", value: "tauri.conf.json", zone: "project_meta" },
  { type: "basename", value: "tsconfig.json", zone: "project_meta" },
  // Tests beat crate-based zones.
  { type: "suffix", value: "_test.rs", zone: "test" },
  { type: "suffix", value: ".test.ts", zone: "test" },
  { type: "suffix", value: ".test.tsx", zone: "test" },
  { type: "suffix", value: ".spec.ts", zone: "test" },
  { type: "suffix", value: ".spec.tsx", zone: "test" },
  { type: "suffix", value: "_test.go", zone: "test" },
  { type: "contains", value: "/tests/", zone: "test" },
  { type: "contains", value: "/__tests__/", zone: "test" },
  // Migrations.
  { type: "contains", value: "/migrations/", zone: "migration" },
  // Docs.
  { type: "prefix", value: ".context/", zone: "docs" },
  { type: "suffix", value: ".md", zone: "docs" },
  { type: "basename", value: "README", zone: "docs" },
  // Desktop UI / shell.
  { type: "prefix", value: "apps/desktop/src-tauri/", zone: "shell" },
  { type: "prefix", value: "apps/desktop/src/", zone: "ui" },
  { type: "prefix", value: "apps/desktop/", zone: "ui" },
  // Crates.
  { type: "prefix", value: "crates/oxplow-tauri-ipc/", zone: "ipc" },
  { type: "prefix", value: "crates/oxplow-domain/", zone: "domain" },
  { type: "prefix", value: "crates/oxplow-db/", zone: "store" },
  { type: "prefix", value: "crates/oxplow-git/", zone: "git" },
  { type: "prefix", value: "crates/oxplow-lsp-installer/", zone: "lsp" },
  { type: "prefix", value: "crates/oxplow-lsp/", zone: "lsp" },
  { type: "prefix", value: "crates/oxplow-runtime/", zone: "runtime" },
  { type: "prefix", value: "crates/oxplow-fs-watch/", zone: "fs_watch" },
  { type: "prefix", value: "crates/oxplow-tmux/", zone: "terminal" },
  { type: "prefix", value: "crates/oxplow-pty/", zone: "terminal" },
  { type: "prefix", value: "crates/oxplow-mcp/", zone: "mcp" },
  { type: "prefix", value: "crates/oxplow-app/", zone: "app_orchestration" },
  { type: "prefix", value: "crates/oxplow-config/", zone: "config" },
  { type: "prefix", value: "crates/oxplow-session/", zone: "session" },
  { type: "prefix", value: "crates/oxplow-plugin/", zone: "plugin" },
  { type: "prefix", value: "crates/oxplow-control-plane/", zone: "plugin" },
  { type: "prefix", value: "crates/oxplow-code-metrics/", zone: "analysis" },
  { type: "prefix", value: "crates/oxplow-code-dup/", zone: "analysis" },
  { type: "prefix", value: "crates/oxplow-code-deps/", zone: "analysis" },
  { type: "prefix", value: "crates/oxplow-tree-source/", zone: "analysis" },
  // Fallback for .toml outside the recognized crates.
  { type: "suffix", value: ".toml", zone: "project_meta" },
];

export function classifyZone(path: string): Zone {
  const normalized = path.replace(/\\/g, "/");
  const slash = normalized.lastIndexOf("/");
  const basename = slash === -1 ? normalized : normalized.slice(slash + 1);
  for (const rule of RULES) {
    switch (rule.type) {
      case "basename":
        if (basename === rule.value) return rule.zone;
        break;
      case "prefix":
        if (normalized.startsWith(rule.value)) return rule.zone;
        break;
      case "contains":
        if (normalized.includes(rule.value)) return rule.zone;
        break;
      case "suffix":
        if (basename.endsWith(rule.value)) return rule.zone;
        break;
    }
  }
  return "other";
}

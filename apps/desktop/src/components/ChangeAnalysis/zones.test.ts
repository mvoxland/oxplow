import { describe, expect, test } from "bun:test";
import { classifyZone } from "./zones";

describe("classifyZone", () => {
  test("ui paths", () => {
    expect(classifyZone("apps/desktop/src/components/Foo.tsx")).toBe("ui");
    expect(classifyZone("apps/desktop/src/stores/tabs.ts")).toBe("ui");
  });

  test("shell takes priority over ui inside desktop", () => {
    expect(classifyZone("apps/desktop/src-tauri/src/main.rs")).toBe("shell");
  });

  test("crates map to their zones", () => {
    expect(classifyZone("crates/oxplow-db/src/lib.rs")).toBe("store");
    expect(classifyZone("crates/oxplow-git/src/blame.rs")).toBe("git");
    expect(classifyZone("crates/oxplow-tauri-ipc/src/lib.rs")).toBe("ipc");
    expect(classifyZone("crates/oxplow-domain/src/work.rs")).toBe("domain");
    expect(classifyZone("crates/oxplow-runtime/src/lib.rs")).toBe("runtime");
    expect(classifyZone("crates/oxplow-lsp/src/lib.rs")).toBe("lsp");
    expect(classifyZone("crates/oxplow-lsp-installer/src/lib.rs")).toBe("lsp");
    expect(classifyZone("crates/oxplow-code-deps/src/zones.rs")).toBe("analysis");
  });

  test("test files beat crate zone", () => {
    expect(classifyZone("crates/oxplow-db/tests/integration.rs")).toBe("test");
    expect(classifyZone("apps/desktop/src/components/Foo.test.tsx")).toBe("test");
  });

  test("migrations classify uniformly", () => {
    expect(classifyZone("crates/oxplow-db/migrations/V001__init.sql")).toBe(
      "migration",
    );
  });

  test("docs", () => {
    expect(classifyZone(".context/architecture.md")).toBe("docs");
    expect(classifyZone("README.md")).toBe("docs");
    expect(classifyZone("README")).toBe("docs");
  });

  test("project meta basenames win regardless of location", () => {
    expect(classifyZone("Cargo.toml")).toBe("project_meta");
    expect(classifyZone("apps/desktop/package.json")).toBe("project_meta");
    expect(classifyZone("apps/desktop/src-tauri/tauri.conf.json")).toBe(
      "project_meta",
    );
  });

  test("windows separators normalize", () => {
    expect(classifyZone("apps\\desktop\\src\\App.tsx")).toBe("ui");
  });

  test("unknown paths fall to other", () => {
    expect(classifyZone("scripts/build.sh")).toBe("other");
  });
});

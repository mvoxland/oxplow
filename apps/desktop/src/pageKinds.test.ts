import { describe, expect, test } from "bun:test";

import { kindForTabId, pageKindIconComponent } from "./pageKinds.js";

describe("kindForTabId", () => {
  test("scheme-prefixed ids return the prefix", () => {
    expect(kindForTabId("file:src/foo.ts")).toBe("file");
    expect(kindForTabId("wiki:url-schemes")).toBe("wiki");
    expect(kindForTabId("task:42")).toBe("task");
    expect(kindForTabId("dir:src/components")).toBe("dir");
    expect(kindForTabId("git-commit:abcdef0")).toBe("git-commit");
    expect(kindForTabId("git-commit:abc:scope:value")).toBe("git-commit");
    expect(kindForTabId("dashboard:planning")).toBe("dashboard");
    expect(kindForTabId("external-url:https://example.com")).toBe("external-url");
    expect(kindForTabId("finding:fnd-1")).toBe("finding");
  });

  test("literal index ids return themselves", () => {
    expect(kindForTabId("agent")).toBe("agent");
    expect(kindForTabId("tasks")).toBe("tasks");
    expect(kindForTabId("done-work")).toBe("done-work");
    expect(kindForTabId("wiki-index")).toBe("wiki-index");
    expect(kindForTabId("files")).toBe("files");
    expect(kindForTabId("settings")).toBe("settings");
    expect(kindForTabId("uncommitted-changes")).toBe("uncommitted-changes");
  });

  test("uncommitted-changes with scope suffix still resolves to the kind", () => {
    expect(kindForTabId("uncommitted-changes:dir:src")).toBe("uncommitted-changes");
  });

  test("unknown bare ids return themselves rather than null", () => {
    expect(kindForTabId("totally-new-page")).toBe("totally-new-page");
  });
});

describe("pageKindIconComponent", () => {
  test("returns an icon for every supported scheme kind", () => {
    const supported = [
      "file",
      "directory",
      "wiki",
      "task",
      "finding",
      "git-commit",
      "diff",
      "duplicate-block",
      "dashboard",
      "op-error",
      "stream-settings",
      "thread-settings",
      "settings",
      "external-url",
      "uncommitted-changes",
      "agent",
      "tasks",
      "done-work",
      "backlog",
      "archived",
      "wiki-index",
      "files",
      "code-quality",
      "local-history",
      "git-history",
      "git-dashboard",
      "hook-events",
      "subsystem-docs",
      "new-stream",
      "new-task",
      "closed-threads",
      "snapshot",
    ];
    for (const k of supported) {
      expect(pageKindIconComponent(k)).not.toBeNull();
    }
  });

  test("display-label aliases passed by <Page kind='...'> resolve", () => {
    expect(pageKindIconComponent("wiki page")).not.toBeNull();
    expect(pageKindIconComponent("commit")).not.toBeNull();
    expect(pageKindIconComponent("new tasks")).not.toBeNull();
    expect(pageKindIconComponent("threads")).not.toBeNull();
  });

  test("unknown kinds return null", () => {
    expect(pageKindIconComponent("nope")).toBeNull();
    expect(pageKindIconComponent("")).toBeNull();
  });
});

import { describe, expect, it } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { FileTree, type FileTreeItem } from "./FileTree.js";

function snapshot(items: FileTreeItem<string>[]): string {
  return renderToStaticMarkup(
    <FileTree items={items} renderItem={(it) => <span>{it.path}</span>} testId="t" />,
  );
}

describe("FileTree", () => {
  it("renders an empty placeholder when given no items", () => {
    const html = snapshot([]);
    expect(html).toContain("No files");
    expect(html).toContain("Expand all");
    expect(html).toContain("Collapse all");
  });

  it("groups files into directories and shows leaf counts", () => {
    const html = snapshot([
      { path: "src/a.ts", data: "a" },
      { path: "src/b.ts", data: "b" },
      { path: "src/sub/c.ts", data: "c" },
      { path: "README.md", data: "r" },
    ]);
    // Top-level dir name + its leaf count (3 files under src/).
    expect(html).toContain(">src<");
    expect(html).toContain(">3<");
    // Nested dir name + leaf count.
    expect(html).toContain(">sub<");
    expect(html).toContain(">1<");
    // Files all rendered.
    expect(html).toContain("src/a.ts");
    expect(html).toContain("src/b.ts");
    expect(html).toContain("src/sub/c.ts");
    expect(html).toContain("README.md");
  });

  it("renders a search input and toolbar buttons", () => {
    const html = snapshot([{ path: "x.ts", data: "x" }]);
    expect(html).toContain('placeholder="Search files…"');
    expect(html).toContain("Expand all");
    expect(html).toContain("Collapse all");
  });
});

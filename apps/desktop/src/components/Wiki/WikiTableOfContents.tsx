import { useEffect, useState } from "react";

interface TocEntry {
  el: HTMLElement;
  text: string;
  level: 2 | 3;
}

/**
 * Table-of-contents rail module for wiki pages. Walks the rendered
 * markdown body for h2/h3 nodes, highlights the section currently under
 * the scroll viewport, and scrolls the host on click. `scrollHost` is
 * the scrollable container that holds the rendered markdown;
 * `bodyText` drives the re-scan when the markdown source changes.
 */
export function WikiTableOfContents({ bodyText, scrollHost }: {
  bodyText: string;
  scrollHost: HTMLElement | null;
}) {
  const [entries, setEntries] = useState<TocEntry[]>([]);
  const [activeIndex, setActiveIndex] = useState<number>(0);

  useEffect(() => {
    if (!scrollHost) {
      setEntries([]);
      return;
    }
    // Defer one frame so MarkdownView's mermaid/code-block effects have
    // a chance to run; not strictly required for h2/h3 but cheap and
    // avoids races on tab switch.
    let cancelled = false;
    const handle = requestAnimationFrame(() => {
      if (cancelled) return;
      const found = Array.from(scrollHost.querySelectorAll<HTMLElement>("h2, h3"));
      setEntries(found.map((el): TocEntry => ({
        el,
        text: el.textContent?.trim() ?? "",
        level: el.tagName === "H2" ? 2 : 3,
      })).filter((e) => e.text.length > 0));
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(handle);
    };
  }, [scrollHost, bodyText]);

  useEffect(() => {
    if (!scrollHost || entries.length === 0) {
      setActiveIndex(0);
      return;
    }
    const compute = () => {
      const hostRect = scrollHost.getBoundingClientRect();
      let current = 0;
      for (let i = 0; i < entries.length; i++) {
        const top = entries[i]!.el.getBoundingClientRect().top - hostRect.top;
        if (top - 8 <= 0) current = i;
        else break;
      }
      setActiveIndex(current);
    };
    compute();
    const onScroll = () => compute();
    scrollHost.addEventListener("scroll", onScroll, { passive: true });
    return () => scrollHost.removeEventListener("scroll", onScroll);
  }, [scrollHost, entries]);

  if (entries.length === 0) return null;

  return (
    <nav
      data-testid="wiki-toc"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 2,
        fontSize: "var(--text-xs)",
      }}
      aria-label="On this page"
    >
      <div style={{
        fontSize: 10,
        textTransform: "uppercase",
        letterSpacing: "0.06em",
        color: "var(--text-muted)",
        marginBottom: 6,
      }}>On this page</div>
      {entries.map((e, i) => {
        const isActive = i === activeIndex;
        return (
          <button
            key={i}
            type="button"
            onClick={() => {
              if (!scrollHost) return;
              const hostRect = scrollHost.getBoundingClientRect();
              const top = e.el.getBoundingClientRect().top - hostRect.top + scrollHost.scrollTop - 8;
              scrollHost.scrollTo({ top, behavior: "smooth" });
            }}
            title={e.text}
            style={{
              background: "transparent",
              border: "none",
              padding: "3px 0",
              textAlign: "left",
              color: isActive ? "var(--text-primary)" : "var(--text-secondary)",
              borderLeft: isActive ? "2px solid var(--accent)" : "2px solid transparent",
              cursor: "pointer",
              fontWeight: isActive ? 600 : 400,
              fontFamily: "var(--font-ui)",
              fontSize: "var(--text-xs)",
              lineHeight: 1.4,
              marginLeft: -2,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            <span style={{ paddingLeft: e.level === 3 ? 20 : 8 }}>{e.text}</span>
          </button>
        );
      })}
    </nav>
  );
}

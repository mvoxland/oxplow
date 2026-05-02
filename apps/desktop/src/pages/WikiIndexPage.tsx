import type { Stream } from "../tauri-bridge/index.js";
import { Page } from "../tabs/Page.js";
import { WikiPane } from "../components/Wiki/WikiPane.js";

export interface WikiIndexPageProps {
  stream: Stream | null;
  selectedSlug: string | null;
  onOpenNote: (slug: string) => void;
}

/**
 * Thin Page wrapper around the existing WikiPane (wiki notes index).
 */
export function WikiIndexPage({ stream, selectedSlug, onOpenNote }: WikiIndexPageProps) {
  return (
    <Page testId="page-wiki-index" title="Notes">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <WikiPane stream={stream} selectedSlug={selectedSlug} onOpenNote={onOpenNote} />
      </div>
    </Page>
  );
}

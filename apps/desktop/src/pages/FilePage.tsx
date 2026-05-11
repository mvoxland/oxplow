import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { EditorPane } from "../components/EditorPane.js";
import { usePageTitle, useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { fileRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";

export interface FilePageProps extends ComponentProps<typeof EditorPane> {
  /** True when the file's draft differs from saved content. Drives the
   *  ● dirty marker on the page title. */
  dirty: boolean;
}

/**
 * Thin Page wrapper around `EditorPane` so file tabs share the same
 * browser-style chrome as every other non-agent tab. The title comes
 * from the file's basename + dirty marker via `usePageTitle`.
 *
 * EditorPane keeps owning all of its internal toolbar / Monaco
 * decorations / blame overlay — the chrome only adds the title row +
 * optional nav bar above it. Backlinks come from the unified
 * `page_ref` graph: every wiki page, tasks, commit, or finding
 * that references this file's path appears in the dropdown.
 * Outbound is generally empty for files (the file itself doesn't
 * point at other pages today), but the slot is wired so a future
 * import-graph extractor would surface immediately.
 */
export function FilePage({ dirty, ...editorProps }: FilePageProps) {
  const path = editorProps.filePath ?? "";
  const basename = path.split("/").pop() ?? path;
  usePageTitle(basename ? `${dirty ? "● " : ""}${basename}` : "");
  const ctxNav = useOptionalPageNavigation();
  const ref = fileRef(path);
  const backlinkEntries = useBacklinks(ref);
  const outboundEntries = usePageOutbound(ref);
  const onOpen = (r: Parameters<NonNullable<typeof ctxNav>["navigate"]>[0]) =>
    ctxNav?.navigate(r);
  const backlinks =
    backlinkEntries.length > 0
      ? {
          count: backlinkEntries.length,
          body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpen} />,
        }
      : undefined;
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} onOpenPage={onOpen} />,
        }
      : undefined;
  return (
    <Page testId="page-file" kind="file" backlinks={backlinks} outbound={outbound}>
      <EditorPane {...editorProps} />
    </Page>
  );
}

import Link from "@tiptap/extension-link";

/**
 * Tiptap Link mark extended to permit our internal URL schemes
 * (`file:`, `dir:`, `gitcommit:`). Tiptap's default `Link` allowlists
 * http/https/mailto/ftp; without this extension our schemes either
 * get stripped on parse or fail the click-validation in the standard
 * link plugin.
 *
 * Click handling for these schemes still happens at the React layer
 * (the `RichTextField` editor surface wires `editorProps.handleClickOn`
 * to route to `useOptionalPageNavigation`), so the mark itself only
 * needs to *preserve* the URL through parse/serialize.
 */
export const InternalLink = Link.extend({
  // Allow our schemes through the URL sanitizer.
  addOptions() {
    return {
      ...this.parent?.(),
      openOnClick: false,
      autolink: false,
      protocols: ["file", "dir", "gitcommit"],
    };
  },
});

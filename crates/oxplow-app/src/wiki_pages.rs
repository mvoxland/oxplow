//! Wiki-note disk sync + backlinks helpers.
//!
//! Bodies live as `.oxplow/wiki/<slug>.md`. The metadata row in
//! `wiki_page` is derived from the file (title, file refs, related
//! notes, body excerpt) by [`sync_from_disk`]. This module is the
//! pure parser + sync layer; the fs watcher in
//! [`crate::wiki_pages_watch`] drives it on file changes.
//!
//! Two ref shapes are extracted:
//!
//! 1. **`[[wikilinks]]`** — preferred form. The interior matches:
//!    - `path/with/slash.ext[:line]` → file ref
//!    - `bare-slug` (kebab-case, no slash, no extension) → related-note ref
//!    Custom display text after `|` is stripped (`[[a/b.ts|label]]`).
//! 2. **Inline file paths** — fallback for legacy notes that didn't
//!    use the `[[…]]` syntax. At least one slash + a 1-6 char extension,
//!    not preceded by `/` or alphanumerics so we don't pick up partial
//!    URLs.
//!
//! Mirrors `src/persistence/wiki-note-refs.ts` and
//! `src/git/notes-watch.ts` from main.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use oxplow_db::{SqliteWikiPageStore, WikiPage};
use oxplow_domain::{DomainError, Timestamp};

/// Tree version a wikilink references. Mirrors the cross-cutting
/// `oxplow_tree_source::TreeVersion` shape; defined inline here so
/// the wiki parser doesn't pull in tree-source as a hard dep, but
/// the variants and serde tags line up so consumers can convert
/// freely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WikiVersion {
    /// Working-tree version. The "local" version in user terms — the
    /// file as it sits on disk right now, possibly with uncommitted
    /// edits. Authored as `@disk` in the wikilink.
    Disk,
    /// A git ref: sha, branch, tag, or `HEAD`. Authored as
    /// `@<spec>` in the wikilink.
    Ref(String),
}

/// One file reference parsed out of a wikilink. The version captures
/// the author's intent at write time: `@disk` says "the working tree
/// when this note was written," `@<sha>` pins a specific committed
/// version. A bare `[[path]]` is treated as `Disk` for back-compat
/// with notes written before the syntax existed.
#[derive(Debug, Clone, PartialEq)]
pub struct WikiFileRef {
    pub path: String,
    pub version: WikiVersion,
    pub line: Option<u32>,
}

/// Refs extracted from a note body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedRefs {
    /// Workspace-relative file paths (`src/foo.ts`), version-stripped
    /// for backlinks lookup. The DB stores this list — backlinks
    /// match by path regardless of which version a note pinned.
    pub file_refs: Vec<String>,
    /// Rich form for renderers and version-aware consumers. Each
    /// entry carries the path + intended version + optional line
    /// anchor, exactly as written in the markdown body.
    pub file_refs_detail: Vec<WikiFileRef>,
    /// Workspace-relative directory paths (`src/components`). Source
    /// form in markdown is `[[dir:src/components]]` — the `dir:`
    /// prefix is the explicit directory marker (mirrors `git:` for
    /// commit refs). A trailing `/` on the path is tolerated and
    /// stripped.
    pub dir_refs: Vec<String>,
    /// Slugs of other wiki pages (`work-item-lifecycle`).
    pub related_notes: Vec<String>,
}

/// Parse a wikilink interior of the form `path[@version][:line]` into
/// a structured `WikiFileRef`, or `None` if the interior doesn't look
/// like a file path. The version slot is `@disk` (working tree) or
/// `@<git-ref>` (sha / branch / tag / `HEAD`); a bare interior with
/// no `@` defaults to `Disk` so legacy notes don't break.
pub fn parse_wiki_file_ref(interior: &str) -> Option<WikiFileRef> {
    let trimmed = interior.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Split off the `@<version>` segment first; the version may
    // contain `:` (e.g. submodule sha), so we can't naively split on
    // `:` for the line anchor without doing version first.
    let (path_and_line, version) = match trimmed.split_once('@') {
        Some((path_part, version_part)) => {
            // The line anchor lives on the version side: `path@<v>:42`.
            let (v, line_part) = match version_part.split_once(':') {
                Some((v, l)) => (v, Some(l)),
                None => (version_part, None),
            };
            let v = v.trim();
            let parsed_version =
                if v.eq_ignore_ascii_case("disk") || v.eq_ignore_ascii_case("local") {
                    WikiVersion::Disk
                } else if !v.is_empty() {
                    WikiVersion::Ref(v.to_string())
                } else {
                    WikiVersion::Disk
                };
            // Reattach the line anchor (if any) to the path so the
            // existing line-stripping path below still applies.
            let pl = match line_part {
                Some(l) => format!("{path_part}:{l}"),
                None => path_part.to_string(),
            };
            (pl, parsed_version)
        }
        None => (trimmed.to_string(), WikiVersion::Disk),
    };

    // Strip the `:line` anchor.
    let (bare, line) = match path_and_line.rsplit_once(':') {
        Some((p, l)) if l.chars().all(|c| c.is_ascii_digit()) && !l.is_empty() => {
            (p.to_string(), l.parse::<u32>().ok())
        }
        _ => (path_and_line.clone(), None),
    };
    if bare.is_empty() || !looks_like_file(&bare) {
        return None;
    }
    Some(WikiFileRef {
        path: bare,
        version,
        line,
    })
}

/// Parse `[[…]]` wikilinks + inline file paths out of `body`.
pub fn parse_refs(body: &str) -> ParsedRefs {
    if body.is_empty() {
        return ParsedRefs::default();
    }
    let mut files = BTreeSet::new();
    let mut dirs = BTreeSet::new();
    let mut notes = BTreeSet::new();
    // Detail entries preserve insertion order (first-seen wins on
    // duplicates) so the renderer can show them in author order.
    let mut details: Vec<WikiFileRef> = Vec::new();
    let mut details_seen: BTreeSet<(String, String, Option<u32>)> = BTreeSet::new();

    // 1. [[wikilinks]] first — they take priority, and we want to
    //    avoid double-counting an inline path that's also wrapped.
    for cap in find_wikilinks(body) {
        let interior = cap.split('|').next().unwrap_or(cap).trim();
        if interior.is_empty() {
            continue;
        }
        // Directory form. Directories don't carry @version yet; the
        // `dir:` prefix lives outside the path-and-anchor grammar.
        if let Some(dir) = looks_like_dir(interior) {
            dirs.insert(dir);
            continue;
        }
        // Try the rich file form first. `parse_wiki_file_ref` handles
        // `path@<version>[:line]` and bare `path[:line]`, returning
        // None if the interior doesn't shape like a file.
        if let Some(rich) = parse_wiki_file_ref(interior) {
            files.insert(rich.path.clone());
            let key = (
                rich.path.clone(),
                match &rich.version {
                    WikiVersion::Disk => "disk".to_string(),
                    WikiVersion::Ref(r) => format!("ref:{r}"),
                },
                rich.line,
            );
            if details_seen.insert(key) {
                details.push(rich);
            }
            continue;
        }
        // Not a file — try slug form (`bare-slug`). Strip the line
        // anchor; slugs don't carry versions.
        let bare = interior.split(':').next().unwrap_or(interior);
        if !bare.is_empty() && looks_like_slug(bare) {
            notes.insert(bare.to_string());
        }
        // Drop git-commit refs (`[[abc1234]]` — 7-40 hex) silently;
        // they're for the renderer, not wiki indexing.
    }

    // 2. Inline file paths. These don't carry a version; they're
    //    legacy free-text mentions, treat as Disk.
    let stripped = strip_urls(body);
    for path in find_inline_paths(&stripped) {
        if files.insert(path.clone()) {
            let key = (path.clone(), "disk".into(), None);
            if details_seen.insert(key) {
                details.push(WikiFileRef {
                    path,
                    version: WikiVersion::Disk,
                    line: None,
                });
            }
        }
    }

    ParsedRefs {
        file_refs: files.into_iter().collect(),
        file_refs_detail: details,
        dir_refs: dirs.into_iter().collect(),
        related_notes: notes.into_iter().collect(),
    }
}

/// Read `<projectDir>/.oxplow/wiki/<slug>.md` and upsert the
/// `wiki_page` row. Deletes the row if the file is gone. Idempotent.
pub async fn sync_from_disk(
    project_dir: &Path,
    store: &SqliteWikiPageStore,
    slug: &str,
) -> Result<(), DomainError> {
    let file_path = wiki_pages_dir(project_dir).join(format!("{slug}.md"));
    if !file_path.exists() {
        return store.delete(slug).await;
    }
    let body = fs::read_to_string(&file_path)
        .map_err(|e| DomainError::Invalid(format!("read note {slug}: {e}")))?;
    let title = extract_title(&body, slug);
    let refs = parse_refs(&body);
    let body_size_bytes = body.as_bytes().len() as i64;
    let body_excerpt = body.chars().take(280).collect::<String>();
    let now = Timestamp::now();
    let existing = store.get(slug).await?;
    let created_at = existing
        .as_ref()
        .map(|n| n.created_at.clone())
        .unwrap_or(now.clone());
    let note = WikiPage {
        slug: slug.to_string(),
        title,
        body_path: file_path.to_string_lossy().into_owned(),
        body_excerpt,
        body_size_bytes,
        file_refs: refs.file_refs,
        dir_refs: refs.dir_refs,
        related_notes: refs.related_notes,
        created_at,
        updated_at: now,
    };
    store.upsert(&note).await
}

/// Sync every `.md` file in the notes dir + prune rows for deleted
/// files. Run once at watcher startup.
pub async fn scan_and_sync_all(
    project_dir: &Path,
    store: &SqliteWikiPageStore,
) -> Result<(), DomainError> {
    let dir = wiki_pages_dir(project_dir);
    fs::create_dir_all(&dir).ok();
    let mut on_disk: BTreeSet<String> = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            if let Some(slug) = path.file_stem().and_then(|s| s.to_str()) {
                on_disk.insert(slug.to_string());
            }
        }
    }
    for slug in &on_disk {
        sync_from_disk(project_dir, store, slug).await?;
    }
    let known = store.list().await?;
    for note in known {
        if !on_disk.contains(&note.slug) {
            store.delete(&note.slug).await?;
        }
    }
    Ok(())
}

/// Return all notes whose `file_refs` contains `path`. The query is
/// over the JSON column — fine for the small sizes notes typically
/// have. If this becomes hot, swap for an inverted-index table.
pub async fn backlinks_for_file(
    store: &SqliteWikiPageStore,
    path: &str,
) -> Result<Vec<WikiPage>, DomainError> {
    let all = store.list().await?;
    Ok(all
        .into_iter()
        .filter(|n| n.file_refs.iter().any(|r| r == path))
        .collect())
}

/// Return all notes whose `dir_refs` contains `path` (the path is the
/// directory form *without* the trailing slash, matching how
/// [`parse_refs`] stores it).
pub async fn backlinks_for_dir(
    store: &SqliteWikiPageStore,
    path: &str,
) -> Result<Vec<WikiPage>, DomainError> {
    let needle = path.trim_end_matches('/');
    let all = store.list().await?;
    Ok(all
        .into_iter()
        .filter(|n| n.dir_refs.iter().any(|r| r == needle))
        .collect())
}

/// Return all notes whose `related_notes` contains `slug`.
pub async fn backlinks_for_note(
    store: &SqliteWikiPageStore,
    slug: &str,
) -> Result<Vec<WikiPage>, DomainError> {
    let all = store.list().await?;
    Ok(all
        .into_iter()
        .filter(|n| n.related_notes.iter().any(|r| r == slug))
        .collect())
}

pub fn wiki_pages_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".oxplow").join("wiki")
}

/// One-time on-disk rename. Earlier versions of oxplow stored wiki
/// pages at `<project>/.oxplow/notes/<slug>.md`; the rename to
/// `wiki` (matching the schema + UI nomenclature) requires moving
/// the directory if it exists. Idempotent and safe — if either the
/// new dir already exists or the old one doesn't, it's a no-op.
/// Called once at boot before any wiki-page reads/writes.
pub fn migrate_legacy_notes_dir(project_dir: &Path) {
    let new_dir = wiki_pages_dir(project_dir);
    let legacy_dir = project_dir.join(".oxplow").join("notes");
    if new_dir.exists() {
        return;
    }
    if !legacy_dir.exists() {
        return;
    }
    if let Err(err) = std::fs::rename(&legacy_dir, &new_dir) {
        tracing::warn!(
            error = %err,
            from = %legacy_dir.display(),
            to = %new_dir.display(),
            "failed to migrate legacy .oxplow/notes -> .oxplow/wiki",
        );
    }
}

fn extract_title(body: &str, fallback: &str) -> String {
    for line in body.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Find every `[[…]]` interior. Naive scan; handles balanced pairs
/// only — we don't need nested wikilinks.
fn find_wikilinks(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Find the closing `]]`.
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    if let Ok(interior) = std::str::from_utf8(&bytes[start..j]) {
                        out.push(interior);
                    }
                    i = j + 2;
                    break;
                }
                j += 1;
            }
            if j + 1 >= bytes.len() {
                break;
            }
            continue;
        }
        i += 1;
    }
    out
}

/// If `s` is a directory wikilink target (`dir:<path>`), return the
/// stripped path. Otherwise return None. The `dir:` prefix is the
/// explicit directory marker — mirrors `git:` for commit refs.
fn looks_like_dir(s: &str) -> Option<String> {
    let trimmed = s.trim();
    let raw = trimmed.strip_prefix("dir:")?.trim_start();
    let bare = raw.trim_end_matches('/');
    if bare.is_empty() || bare.contains('\n') || bare.contains('|') {
        return None;
    }
    // Reject double-slash sequences (`//`), absolute paths, and URL
    // tails — directory refs are always workspace-relative.
    if bare.starts_with('/') || bare.contains("//") {
        return None;
    }
    Some(bare.to_string())
}

fn looks_like_file(s: &str) -> bool {
    if !s.contains('/') {
        return false;
    }
    // Trailing extension 1-6 chars after the last dot.
    if let Some(dot) = s.rfind('.') {
        let ext = &s[dot + 1..];
        if (1..=6).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return true;
        }
    }
    false
}

fn looks_like_slug(s: &str) -> bool {
    if s.is_empty() || s.len() > 80 {
        return false;
    }
    if s.contains('/') || s.contains('.') || s.contains(' ') {
        return false;
    }
    // Skip git commit hashes (7-40 hex), they go to the renderer
    // not the wiki index.
    if matches!(s.len(), 7..=40) && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn strip_urls(body: &str) -> String {
    // Replace URL-shaped runs with a space so the inline-path scan
    // doesn't pick up `https://example.com/path.json`. Cheap state
    // machine; full URL grammar isn't needed.
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for `<scheme>://`.
        if i + 3 < bytes.len() && bytes[i..i + 3] == *b"://" {
            // Skip back to the start of the scheme word and forward
            // through the URL.
            // (We've already emitted the chars before `://`; rewind out.)
            for _ in 0..out
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphabetic() || matches!(*c, '+' | '-' | '.'))
                .count()
            {
                out.pop();
            }
            i += 3;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(' ');
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn find_inline_paths(body: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    // Tokenize on whitespace + a few punctuation chars; check each
    // token for "looks like path/to/file.ext".
    let separators: &[char] = &[
        ' ', '\t', '\n', '\r', ',', ';', '(', ')', '[', ']', '"', '\'',
    ];
    for token in body.split(|c: char| separators.contains(&c)) {
        let trimmed = token.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':'));
        if trimmed.is_empty() || trimmed.starts_with('/') {
            continue;
        }
        // Must look like file AND not be a single dotted token.
        if looks_like_file(trimmed) && !trimmed.starts_with("//") {
            // Strip any trailing :Symbol anchor.
            let bare = trimmed.split(':').next().unwrap_or(trimmed);
            out.insert(bare.to_string());
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_wikilink_files() {
        let refs = parse_refs("see [[src/foo.ts]] and [[src/bar.tsx:42]]");
        assert_eq!(refs.file_refs, vec!["src/bar.tsx", "src/foo.ts"]);
        assert!(refs.related_notes.is_empty());
    }

    #[test]
    fn parse_extracts_wikilink_notes() {
        let refs = parse_refs("related: [[work-item-lifecycle]] and [[stop-hook-pipeline]]");
        assert_eq!(
            refs.related_notes,
            vec!["stop-hook-pipeline", "work-item-lifecycle"]
        );
    }

    #[test]
    fn parse_strips_display_text() {
        let refs = parse_refs("[[src/foo.ts|the foo helper]]");
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
    }

    #[test]
    fn parse_extracts_directory_refs() {
        let refs = parse_refs("see [[dir:src/components]] for the buttons");
        assert_eq!(refs.dir_refs, vec!["src/components"]);
        assert!(refs.file_refs.is_empty());
        assert!(refs.related_notes.is_empty());
    }

    #[test]
    fn parse_directory_ref_with_label() {
        let refs = parse_refs("[[dir:src/components|the components folder]]");
        assert_eq!(refs.dir_refs, vec!["src/components"]);
    }

    #[test]
    fn parse_directory_ref_tolerates_trailing_slash() {
        let refs = parse_refs("[[dir:src/foo/]]");
        assert_eq!(refs.dir_refs, vec!["src/foo"]);
    }

    #[test]
    fn parse_directory_ref_dedupes() {
        let refs = parse_refs("[[dir:src/foo]] then [[dir:src/foo]] and [[dir:src/bar/baz]]");
        assert_eq!(refs.dir_refs, vec!["src/bar/baz", "src/foo"]);
    }

    #[test]
    fn parse_keeps_file_form_without_dir_prefix() {
        // Regression: `[[src/foo.ts]]` must remain a file ref even
        // after the directory branch was added.
        let refs = parse_refs("[[src/foo.ts]]");
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
        assert!(refs.dir_refs.is_empty());
    }

    #[test]
    fn parse_skips_commit_hashes() {
        let refs = parse_refs("see [[abc1234]] and [[abc1234567890ab]]");
        assert!(refs.file_refs.is_empty());
        assert!(refs.related_notes.is_empty());
    }

    #[test]
    fn parse_picks_up_inline_paths() {
        let refs = parse_refs("the file src/foo.ts has the bug");
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
    }

    #[test]
    fn parse_skips_urls() {
        let refs = parse_refs("see https://example.com/path.json for details");
        assert!(refs.file_refs.is_empty());
    }

    #[test]
    fn parse_wikilink_with_disk_version_emits_disk_in_detail() {
        let refs = parse_refs("see [[src/foo.ts@disk]] for the live version");
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
        assert_eq!(refs.file_refs_detail.len(), 1);
        let d = &refs.file_refs_detail[0];
        assert_eq!(d.path, "src/foo.ts");
        assert_eq!(d.version, WikiVersion::Disk);
        assert_eq!(d.line, None);
    }

    #[test]
    fn parse_wikilink_with_local_alias_treated_as_disk() {
        // `@local` is accepted as an alias for `@disk` because the
        // user-facing terminology in the capture skill says "local
        // version" — both must round-trip to the same WikiVersion.
        let refs = parse_refs("[[src/foo.ts@local]]");
        assert_eq!(refs.file_refs_detail[0].version, WikiVersion::Disk);
    }

    #[test]
    fn parse_wikilink_with_sha_version() {
        let refs = parse_refs("[[crates/oxplow-app/src/lib.rs@abc1234]]");
        let d = &refs.file_refs_detail[0];
        assert_eq!(d.path, "crates/oxplow-app/src/lib.rs");
        assert_eq!(d.version, WikiVersion::Ref("abc1234".into()));
    }

    #[test]
    fn parse_wikilink_with_head_version() {
        let refs = parse_refs("[[src/foo.ts@HEAD]]");
        assert_eq!(
            refs.file_refs_detail[0].version,
            WikiVersion::Ref("HEAD".into())
        );
    }

    #[test]
    fn parse_wikilink_with_branch_version() {
        let refs = parse_refs("[[src/foo.ts@main]]");
        assert_eq!(
            refs.file_refs_detail[0].version,
            WikiVersion::Ref("main".into())
        );
    }

    #[test]
    fn parse_wikilink_version_with_line_anchor() {
        let refs = parse_refs("[[src/foo.ts@HEAD:42]]");
        let d = &refs.file_refs_detail[0];
        assert_eq!(d.path, "src/foo.ts");
        assert_eq!(d.version, WikiVersion::Ref("HEAD".into()));
        assert_eq!(d.line, Some(42));
    }

    #[test]
    fn parse_wikilink_bare_path_defaults_to_disk() {
        let refs = parse_refs("[[src/foo.ts]]");
        let d = &refs.file_refs_detail[0];
        assert_eq!(d.version, WikiVersion::Disk);
        assert_eq!(d.line, None);
    }

    #[test]
    fn parse_wikilink_bare_with_line() {
        let refs = parse_refs("[[src/foo.ts:42]]");
        let d = &refs.file_refs_detail[0];
        assert_eq!(d.line, Some(42));
        assert_eq!(d.version, WikiVersion::Disk);
    }

    #[test]
    fn parse_wikilink_strips_version_from_backlinks_path() {
        // The DB-stored `file_refs` path list MUST be version-stripped
        // so backlinks_for_file("src/foo.ts") matches notes that pinned
        // any version of foo.ts. The detail list keeps the version.
        let refs = parse_refs(
            "see [[src/foo.ts@HEAD]] and [[src/foo.ts@disk]] and [[src/foo.ts@abc1234]]",
        );
        assert_eq!(refs.file_refs, vec!["src/foo.ts"]);
        assert_eq!(refs.file_refs_detail.len(), 3);
    }

    #[test]
    fn extract_title_picks_first_h1() {
        assert_eq!(extract_title("# Hello\n\nbody", "fallback"), "Hello");
        assert_eq!(extract_title("no heading", "fallback"), "fallback");
    }
}

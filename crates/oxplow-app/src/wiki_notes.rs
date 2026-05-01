//! Wiki-note disk sync + backlinks helpers.
//!
//! Bodies live as `.oxplow/notes/<slug>.md`. The metadata row in
//! `wiki_note` is derived from the file (title, file refs, related
//! notes, body excerpt) by [`sync_from_disk`]. This module is the
//! pure parser + sync layer; the fs watcher in
//! [`crate::wiki_notes_watch`] drives it on file changes.
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

use oxplow_db::{SqliteWikiNoteStore, WikiNote};
use oxplow_domain::{DomainError, Timestamp};

/// Both kinds of refs extracted from a note body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedRefs {
    /// Workspace-relative file paths (`src/foo.ts`).
    pub file_refs: Vec<String>,
    /// Slugs of other wiki notes (`work-item-lifecycle`).
    pub related_notes: Vec<String>,
}

/// Parse `[[…]]` wikilinks + inline file paths out of `body`.
pub fn parse_refs(body: &str) -> ParsedRefs {
    if body.is_empty() {
        return ParsedRefs::default();
    }
    let mut files = BTreeSet::new();
    let mut notes = BTreeSet::new();

    // 1. [[wikilinks]] first — they take priority, and we want to
    //    avoid double-counting an inline path that's also wrapped.
    for cap in find_wikilinks(body) {
        let interior = cap.split('|').next().unwrap_or(cap).trim();
        // Strip trailing `:line` anchor (numeric or symbol).
        let bare = interior.split(':').next().unwrap_or(interior);
        if bare.is_empty() {
            continue;
        }
        if looks_like_file(bare) {
            files.insert(bare.to_string());
        } else if looks_like_slug(bare) {
            notes.insert(bare.to_string());
        }
        // Drop git-commit refs (`[[abc1234]]` — 7-40 hex) silently;
        // they're for the renderer, not wiki indexing.
    }

    // 2. Inline file paths.
    let stripped = strip_urls(body);
    for path in find_inline_paths(&stripped) {
        files.insert(path);
    }

    ParsedRefs {
        file_refs: files.into_iter().collect(),
        related_notes: notes.into_iter().collect(),
    }
}

/// Read `<projectDir>/.oxplow/notes/<slug>.md` and upsert the
/// `wiki_note` row. Deletes the row if the file is gone. Idempotent.
pub async fn sync_from_disk(
    project_dir: &Path,
    store: &SqliteWikiNoteStore,
    slug: &str,
) -> Result<(), DomainError> {
    let file_path = notes_dir(project_dir).join(format!("{slug}.md"));
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
    let created_at = existing.as_ref().map(|n| n.created_at.clone()).unwrap_or(now.clone());
    let note = WikiNote {
        slug: slug.to_string(),
        title,
        body_path: file_path.to_string_lossy().into_owned(),
        body_excerpt,
        body_size_bytes,
        file_refs: refs.file_refs,
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
    store: &SqliteWikiNoteStore,
) -> Result<(), DomainError> {
    let dir = notes_dir(project_dir);
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
    store: &SqliteWikiNoteStore,
    path: &str,
) -> Result<Vec<WikiNote>, DomainError> {
    let all = store.list().await?;
    Ok(all
        .into_iter()
        .filter(|n| n.file_refs.iter().any(|r| r == path))
        .collect())
}

/// Return all notes whose `related_notes` contains `slug`.
pub async fn backlinks_for_note(
    store: &SqliteWikiNoteStore,
    slug: &str,
) -> Result<Vec<WikiNote>, DomainError> {
    let all = store.list().await?;
    Ok(all
        .into_iter()
        .filter(|n| n.related_notes.iter().any(|r| r == slug))
        .collect())
}

pub fn notes_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".oxplow").join("notes")
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
    let separators: &[char] = &[' ', '\t', '\n', '\r', ',', ';', '(', ')', '[', ']', '"', '\''];
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
    fn extract_title_picks_first_h1() {
        assert_eq!(extract_title("# Hello\n\nbody", "fallback"), "Hello");
        assert_eq!(extract_title("no heading", "fallback"), "fallback");
    }
}

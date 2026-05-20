//! Global session store: the set of project directories that
//! currently have an open window.
//!
//! Oxplow is process-per-window, so there's no app-level coordinator
//! that knows "which windows are open". Instead each project process
//! records its own dir here on boot (`add`) and removes it when its
//! window is deliberately closed (`remove`). On a bare launch the
//! startup path reads `list()` and reopens whatever's still present —
//! i.e. the windows that were open at last exit (a clean Cmd-Q / crash
//! / shutdown leaves the entries in place; only an explicit window
//! close removes one).
//!
//! Multiple project processes mutate this file concurrently, so every
//! read-modify-write is wrapped in a cross-process `fs2` exclusive
//! file lock.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionDoc {
    /// Canonical paths of project dirs with a live window.
    #[serde(default)]
    open: Vec<String>,
}

/// Handle to the global `session.json` open-window set.
#[derive(Debug, Clone)]
pub struct SessionProjects {
    json_path: PathBuf,
}

impl SessionProjects {
    pub fn new(json_path: impl Into<PathBuf>) -> Self {
        Self {
            json_path: json_path.into(),
        }
    }

    /// Project dirs currently recorded as open (canonical paths).
    pub fn list(&self) -> Vec<String> {
        self.with_locked(|doc| doc.open.clone()).unwrap_or_default()
    }

    /// Record `dir` as having an open window (dedup by canonical path).
    pub fn add(&self, dir: impl AsRef<Path>) {
        let canonical = canonicalize(dir.as_ref());
        let _ = self.with_locked(|doc| {
            if !doc.open.contains(&canonical) {
                doc.open.push(canonical.clone());
            }
        });
    }

    /// Drop `dir` from the open set (its window was closed).
    pub fn remove(&self, dir: impl AsRef<Path>) {
        let canonical = canonicalize(dir.as_ref());
        let _ = self.with_locked(|doc| {
            doc.open.retain(|p| p != &canonical);
        });
    }

    /// Open (creating) the session file, take an exclusive cross-process
    /// lock, read+parse the doc, run `f` (which may mutate it), and —
    /// if mutated — write it back. Returns `f`'s result. Any IO/parse
    /// failure degrades to a default (empty) doc so a corrupt file never
    /// wedges window tracking.
    fn with_locked<R>(&self, f: impl FnOnce(&mut SessionDoc) -> R) -> std::io::Result<R> {
        if let Some(parent) = self.json_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.json_path)?;
        file.lock_exclusive()?;

        let mut raw = String::new();
        let _ = file.read_to_string(&mut raw);
        let before = raw.clone();
        let mut doc: SessionDoc = serde_json::from_str(&raw).unwrap_or_default();

        let result = f(&mut doc);

        // Only rewrite when the serialized form actually changed.
        if let Ok(after) = serde_json::to_string_pretty(&doc) {
            if after != before {
                let _ = file.set_len(0);
                let _ = file.seek(SeekFrom::Start(0));
                let _ = file.write_all(after.as_bytes());
            }
        }
        let _ = FileExt::unlock(&file);
        Ok(result)
    }
}

/// Canonicalize for stable dedup across symlinks; fall back to the
/// lexical path string if it can't be resolved.
fn canonicalize(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store(dir: &tempfile::TempDir) -> SessionProjects {
        SessionProjects::new(dir.path().join("state").join("session.json"))
    }

    #[test]
    fn missing_file_lists_empty() {
        let dir = tempdir().unwrap();
        assert!(store(&dir).list().is_empty());
    }

    #[test]
    fn add_then_list_then_remove() {
        let dir = tempdir().unwrap();
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let s = store(&dir);

        s.add(a.path());
        s.add(b.path());
        s.add(a.path()); // dedup
        assert_eq!(s.list().len(), 2);

        let canon_a = std::fs::canonicalize(a.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        s.remove(a.path());
        let remaining = s.list();
        assert_eq!(remaining.len(), 1);
        assert!(!remaining.contains(&canon_a));
    }

    #[test]
    fn corrupt_file_degrades_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let s = SessionProjects::new(path);
        assert!(s.list().is_empty());
        // Still writable after a corrupt read.
        let proj = tempdir().unwrap();
        s.add(proj.path());
        assert_eq!(s.list().len(), 1);
    }
}

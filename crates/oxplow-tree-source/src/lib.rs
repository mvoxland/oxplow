//! Versioned tree access.
//!
//! Provides the abstraction that every file-reading subsystem in
//! oxplow should go through: a [`TreeSource`] enumerates files and
//! reads their content at a specific [`TreeVersion`] (working tree,
//! a git ref, or — eventually — a local-history snapshot). A
//! [`FileFilter`] decides which paths from a source make it into
//! whatever consumes them (dup scan, metrics scan, …).
//!
//! The point of routing every read through this trait is correctness:
//! callers must declare which version of the tree they want. There is
//! no implicit "the working tree" default. The duplication scan was
//! the first subsystem to bake the working-tree assumption in; this
//! crate exists so it's the last.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Repo-relative file path. Always normalized to forward slashes.
pub type RepoPath = String;

/// Identifies which version of the tree a `TreeSource` represents.
/// Carried alongside scan results so consumers can re-read the same
/// content without ambiguity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TreeVersion {
    /// Working tree on disk.
    Disk,
    /// A git ref — sha, branch, tag, or `HEAD`.
    Ref { r#ref: String },
    /// A local-history snapshot. Wired through the type system so
    /// callers can match exhaustively, but no source impl ships in
    /// this crate yet — see [`SnapshotTreeSource`].
    Snapshot { id: String },
}

impl TreeVersion {
    /// Stable tag string used in storage / IPC routing.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            TreeVersion::Disk => "disk",
            TreeVersion::Ref { .. } => "ref",
            TreeVersion::Snapshot { .. } => "snapshot",
        }
    }

    /// The version's identifier string, or `None` for `Disk`.
    pub fn value(&self) -> Option<&str> {
        match self {
            TreeVersion::Disk => None,
            TreeVersion::Ref { r#ref } => Some(r#ref.as_str()),
            TreeVersion::Snapshot { id } => Some(id.as_str()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    #[error("io: {0}")]
    Io(String),
    #[error("git: {0}")]
    Git(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

/// A tree of files at a specific version. Implementations enumerate
/// what's there and read content on demand.
pub trait TreeSource: Send + Sync {
    fn version(&self) -> TreeVersion;
    /// All file paths the source can offer, repo-relative + forward
    /// slashes. Implementations apply their own structural skips
    /// (`.git`, `target`, `node_modules`, …); semantic filtering is
    /// the caller's job via [`FileFilter`].
    fn list_files(&self) -> Result<Vec<RepoPath>, TreeError>;
    /// Read content for `path`. `Ok(None)` ⇒ the path doesn't exist
    /// in this tree (or is binary / unreadable — callers don't need
    /// to distinguish). `Err` ⇒ infrastructure failure.
    fn read(&self, path: &str) -> Result<Option<String>, TreeError>;
}

/// Decides whether a path should pass through to the consumer.
pub trait FileFilter: Send + Sync {
    fn keep(&self, path: &str) -> bool;
}

/// Pass-through filter — every path the source enumerates makes it
/// into the corpus.
pub struct AllFiles;

impl FileFilter for AllFiles {
    fn keep(&self, _path: &str) -> bool {
        true
    }
}

/// Restrict the corpus to a known set of paths.
pub struct ExplicitPaths {
    paths: HashSet<RepoPath>,
}

impl ExplicitPaths {
    pub fn new<I, S>(paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<RepoPath>,
    {
        Self {
            paths: paths.into_iter().map(Into::into).collect(),
        }
    }
}

impl FileFilter for ExplicitPaths {
    fn keep(&self, path: &str) -> bool {
        self.paths.contains(path)
    }
}

/// Convenience: enumerate `source` and apply `filter`, reading each
/// kept path. Skips files that read as `None`. Intended for callers
/// that want "give me (path, content) for the corpus" without
/// re-implementing the loop.
pub fn collect_corpus(
    source: &dyn TreeSource,
    filter: &dyn FileFilter,
) -> Result<Vec<(RepoPath, String)>, TreeError> {
    let mut out = Vec::new();
    for path in source.list_files()? {
        if !filter.keep(&path) {
            continue;
        }
        match source.read(&path)? {
            Some(content) => out.push((path, content)),
            None => continue,
        }
    }
    Ok(out)
}

/// Working-tree source. Walks the filesystem under `project_dir`,
/// skipping the usual dotdirs + build folders. `read` does a plain
/// `std::fs::read_to_string`; binary / unreadable files report as
/// `Ok(None)`.
pub struct DiskTreeSource {
    project_dir: PathBuf,
    skip: Vec<&'static str>,
}

impl DiskTreeSource {
    pub fn new(project_dir: impl Into<PathBuf>) -> Self {
        Self {
            project_dir: project_dir.into(),
            skip: vec!["target", "node_modules", "dist", "build", ".git"],
        }
    }
}

impl TreeSource for DiskTreeSource {
    fn version(&self) -> TreeVersion {
        TreeVersion::Disk
    }

    fn list_files(&self) -> Result<Vec<RepoPath>, TreeError> {
        let skip = self.skip.clone();
        let project = self.project_dir.clone();
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(&project).into_iter().filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if e.depth() == 0 {
                return true;
            }
            if name.starts_with('.') && e.file_type().is_dir() {
                return false;
            }
            !(e.file_type().is_dir() && skip.contains(&name.as_ref()))
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = match entry.path().strip_prefix(&project) {
                Ok(p) => p,
                Err(_) => continue,
            };
            out.push(normalize_path(rel));
        }
        Ok(out)
    }

    fn read(&self, path: &str) -> Result<Option<String>, TreeError> {
        let abs = self.project_dir.join(path);
        match std::fs::read_to_string(&abs) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            // InvalidData = non-UTF8; treat the same as "not text".
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Ok(None),
            Err(e) => Err(TreeError::Io(format!("read {path}: {e}"))),
        }
    }
}

/// Git-ref source. Resolves `ref_spec` against `repo_dir` and walks
/// the resulting commit's tree. Read goes through the blob.
pub struct GitTreeSource {
    repo_dir: PathBuf,
    ref_spec: String,
}

impl GitTreeSource {
    pub fn new(repo_dir: impl Into<PathBuf>, ref_spec: impl Into<String>) -> Self {
        Self {
            repo_dir: repo_dir.into(),
            ref_spec: ref_spec.into(),
        }
    }

    fn open_tree(&self) -> Result<(git2::Repository, git2::Oid), TreeError> {
        let repo = git2::Repository::open(&self.repo_dir)
            .map_err(|e| TreeError::Git(format!("open repo: {e}")))?;
        let oid = {
            let obj = repo
                .revparse_single(&self.ref_spec)
                .map_err(|e| TreeError::Git(format!("revparse {}: {}", self.ref_spec, e)))?;
            let commit = obj
                .peel_to_commit()
                .map_err(|e| TreeError::Git(format!("peel_to_commit: {e}")))?;
            commit
                .tree()
                .map_err(|e| TreeError::Git(format!("commit.tree: {e}")))?
                .id()
        };
        Ok((repo, oid))
    }
}

impl TreeSource for GitTreeSource {
    fn version(&self) -> TreeVersion {
        TreeVersion::Ref {
            r#ref: self.ref_spec.clone(),
        }
    }

    fn list_files(&self) -> Result<Vec<RepoPath>, TreeError> {
        let (repo, tree_oid) = self.open_tree()?;
        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| TreeError::Git(format!("find_tree: {e}")))?;
        let mut out = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                let name = entry.name().unwrap_or("");
                let path = if dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{dir}{name}")
                };
                out.push(path);
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(|e| TreeError::Git(format!("tree.walk: {e}")))?;
        Ok(out)
    }

    fn read(&self, path: &str) -> Result<Option<String>, TreeError> {
        let (repo, tree_oid) = self.open_tree()?;
        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| TreeError::Git(format!("find_tree: {e}")))?;
        let entry = match tree.get_path(Path::new(path)) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };
        let object = entry
            .to_object(&repo)
            .map_err(|e| TreeError::Git(format!("entry.to_object: {e}")))?;
        let blob = match object.as_blob() {
            Some(b) => b,
            None => return Ok(None),
        };
        match String::from_utf8(blob.content().to_vec()) {
            Ok(s) => Ok(Some(s)),
            Err(_) => Ok(None),
        }
    }
}

/// Stub for the future local-history snapshot source. Wired so
/// callers can match exhaustively today; every method returns
/// [`TreeError::NotImplemented`] until the snapshot store is plumbed
/// in. Don't construct this in production code paths yet.
pub struct SnapshotTreeSource {
    pub snapshot_id: String,
}

impl SnapshotTreeSource {
    pub fn new(snapshot_id: impl Into<String>) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
        }
    }
}

impl TreeSource for SnapshotTreeSource {
    fn version(&self) -> TreeVersion {
        TreeVersion::Snapshot {
            id: self.snapshot_id.clone(),
        }
    }

    fn list_files(&self) -> Result<Vec<RepoPath>, TreeError> {
        Err(TreeError::NotImplemented("SnapshotTreeSource::list_files"))
    }

    fn read(&self, _path: &str) -> Result<Option<String>, TreeError> {
        Err(TreeError::NotImplemented("SnapshotTreeSource::read"))
    }
}

fn normalize_path(p: &Path) -> RepoPath {
    p.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn write(dir: &Path, rel: &str, body: &str) {
        let abs = dir.join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(abs, body).unwrap();
    }

    #[test]
    fn disk_source_lists_and_reads_files_skipping_dotdirs() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.rs", "fn a() {}\n");
        write(dir.path(), "src/b.rs", "fn b() {}\n");
        write(dir.path(), ".git/HEAD", "ref: refs/heads/main\n");
        write(dir.path(), "target/build_artifact", "ignored\n");
        write(dir.path(), "node_modules/pkg/index.js", "ignored\n");

        let source = DiskTreeSource::new(dir.path());
        let mut listed = source.list_files().unwrap();
        listed.sort();
        assert_eq!(listed, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);

        let body = source.read("src/a.rs").unwrap();
        assert_eq!(body.as_deref(), Some("fn a() {}\n"));
        assert!(source.read("src/missing.rs").unwrap().is_none());
        assert_eq!(source.version(), TreeVersion::Disk);
    }

    #[test]
    fn explicit_paths_filters_correctly() {
        let filter = ExplicitPaths::new(["src/a.rs", "src/b.rs"]);
        assert!(filter.keep("src/a.rs"));
        assert!(!filter.keep("src/c.rs"));

        let all = AllFiles;
        assert!(all.keep("anything"));
    }

    #[test]
    fn collect_corpus_applies_filter() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "fn a() {}\n");
        write(dir.path(), "b.rs", "fn b() {}\n");
        let source = DiskTreeSource::new(dir.path());
        let filter = ExplicitPaths::new(["a.rs"]);
        let corpus = collect_corpus(&source, &filter).unwrap();
        assert_eq!(corpus, vec![("a.rs".to_string(), "fn a() {}\n".to_string())]);
    }

    #[test]
    fn git_source_reads_blob_at_ref_not_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        // git init + commit
        let run = |args: &[&str]| {
            let out = Command::new("git").args(args).current_dir(path).output().unwrap();
            assert!(out.status.success(), "git {args:?} failed: {:?}", out);
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        write(path, "a.rs", "old\n");
        run(&["add", "a.rs"]);
        run(&["commit", "-q", "-m", "first"]);
        // Mutate disk after commit — the GitTreeSource must see the
        // committed version, not the working tree.
        write(path, "a.rs", "new\n");

        let source = GitTreeSource::new(path, "HEAD");
        let listed = source.list_files().unwrap();
        assert_eq!(listed, vec!["a.rs".to_string()]);
        assert_eq!(source.read("a.rs").unwrap().as_deref(), Some("old\n"));
        assert!(source.read("missing.rs").unwrap().is_none());
        assert_eq!(
            source.version(),
            TreeVersion::Ref { r#ref: "HEAD".into() }
        );
    }

    #[test]
    fn snapshot_source_returns_not_implemented() {
        let source = SnapshotTreeSource::new("snap-1");
        assert!(matches!(
            source.list_files(),
            Err(TreeError::NotImplemented(_))
        ));
        assert!(matches!(
            source.read("x"),
            Err(TreeError::NotImplemented(_))
        ));
        assert_eq!(
            source.version(),
            TreeVersion::Snapshot { id: "snap-1".into() }
        );
    }

    #[test]
    fn tree_version_kind_tag_and_value() {
        assert_eq!(TreeVersion::Disk.kind_tag(), "disk");
        assert_eq!(TreeVersion::Disk.value(), None);
        let r = TreeVersion::Ref { r#ref: "abc".into() };
        assert_eq!(r.kind_tag(), "ref");
        assert_eq!(r.value(), Some("abc"));
        let s = TreeVersion::Snapshot { id: "x".into() };
        assert_eq!(s.kind_tag(), "snapshot");
        assert_eq!(s.value(), Some("x"));
    }
}

//! Stream + worktree lifecycle.
//!
//! Encodes the primary-vs-worktree invariant from
//! `.context/architecture.md`: exactly one primary stream per project,
//! everything else is a worktree under `.oxplow/worktrees/<slug>/`.
//!
//! Composes `oxplow-git` (for actual `git worktree add`) and the
//! `StreamStore` trait from `oxplow-domain` for persistence.

pub mod thread_service;
pub use thread_service::{ThreadError, ThreadService};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;
use tracing::info;

use oxplow_domain::stores::StreamStore;
use oxplow_domain::{DomainError, Stream, StreamId, StreamKind, Timestamp};
use oxplow_git::{detect_current_branch, ensure_worktree, is_git_repo, is_git_worktree};

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("workspace is not a git repo: {0}")]
    NotARepo(PathBuf),
    #[error("workspace is a secondary git worktree (refusing to start): {0}")]
    InWorktree(PathBuf),
    #[error("primary stream already exists")]
    PrimaryExists,
    #[error("primary stream missing")]
    PrimaryMissing,
    #[error("worktree slug \"{0}\" already exists for stream {1}")]
    DuplicateWorktreeSlug(String, StreamId),
    #[error("git: {0}")]
    Git(#[from] oxplow_git::EnsureWorktreeError),
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

/// Configuration for stream-management.
#[derive(Debug, Clone)]
pub struct WorkspaceLayout {
    /// Project root — the daemon's start directory.
    pub project_dir: PathBuf,
    /// Where worktree streams live: `<project_dir>/.oxplow/worktrees/`.
    pub worktrees_root: PathBuf,
}

impl WorkspaceLayout {
    pub fn for_project(project_dir: impl Into<PathBuf>) -> Self {
        let project_dir = project_dir.into();
        let worktrees_root = project_dir.join(".oxplow").join("worktrees");
        Self {
            project_dir,
            worktrees_root,
        }
    }
}

/// Top-level service. Cheap to clone — internals are `Arc`'d.
#[derive(Clone)]
pub struct StreamService {
    layout: WorkspaceLayout,
    streams: Arc<dyn StreamStore>,
}

impl StreamService {
    pub fn new(layout: WorkspaceLayout, streams: Arc<dyn StreamStore>) -> Self {
        Self { layout, streams }
    }

    /// Validate the workspace before doing anything else: it must be
    /// a git repo and must NOT be a secondary worktree.
    pub fn validate_workspace(&self) -> Result<(), SessionError> {
        if !is_git_repo(&self.layout.project_dir) {
            return Err(SessionError::NotARepo(self.layout.project_dir.clone()));
        }
        if is_git_worktree(&self.layout.project_dir) {
            return Err(SessionError::InWorktree(self.layout.project_dir.clone()));
        }
        Ok(())
    }

    /// Idempotent: ensures a primary stream exists for this project.
    /// Reuses the existing one if present.
    pub async fn ensure_primary(&self) -> Result<Stream, SessionError> {
        self.validate_workspace()?;
        if let Some(existing) = self.streams.primary().await? {
            return Ok(existing);
        }
        let branch = detect_current_branch(&self.layout.project_dir)
            .unwrap_or_else(|| "HEAD".to_string());
        let now = Timestamp::now();
        let title = self
            .layout
            .project_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "primary".into());
        let stream = Stream {
            id: StreamId::new(),
            kind: StreamKind::Primary,
            title,
            summary: String::new(),
            branch: branch.clone(),
            branch_ref: format!("refs/heads/{branch}"),
            branch_source: branch,
            worktree_path: self.layout.project_dir.to_string_lossy().into_owned(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            created_at: now,
            updated_at: now,
        };
        self.streams.upsert(&stream).await?;
        info!(stream_id = %stream.id, "primary stream created");
        Ok(stream)
    }

    /// Create a worktree stream. Slug is the directory name under
    /// `.oxplow/worktrees/`; it's fixed at creation and never changes.
    /// `branch_source` is the branch to fork from (e.g. `main`).
    pub async fn create_worktree(
        &self,
        slug: &str,
        title: impl Into<String>,
        branch: impl Into<String>,
        branch_source: impl Into<String>,
    ) -> Result<Stream, SessionError> {
        self.validate_workspace()?;
        // Ensure primary exists so the layout invariant holds.
        let _primary = self
            .streams
            .primary()
            .await?
            .ok_or(SessionError::PrimaryMissing)?;

        let branch = branch.into();
        let branch_source = branch_source.into();
        let title = title.into();

        let worktree_path = self.layout.worktrees_root.join(slug);

        // Reject duplicate slug — not on the path, but on a stream
        // already pointing at the same path. (We can't easily query the
        // store by path without a new method, so we check the cheap
        // duplicate case: anything at the path means we punt.)
        if worktree_path.exists() {
            // Scan existing streams for the path; if found, return that
            // stream (idempotent ensure semantics).
            for existing in self.streams.list().await? {
                if existing.worktree_path == worktree_path.to_string_lossy() {
                    return Ok(existing);
                }
            }
            return Err(SessionError::DuplicateWorktreeSlug(
                slug.to_string(),
                StreamId::from(slug),
            ));
        }

        ensure_worktree(
            &self.layout.project_dir,
            &worktree_path,
            &branch,
            &branch_source,
        )?;

        let now = Timestamp::now();
        let stream = Stream {
            id: StreamId::new(),
            kind: StreamKind::Worktree,
            title,
            summary: String::new(),
            branch: branch.clone(),
            branch_ref: format!("refs/heads/{branch}"),
            branch_source,
            worktree_path: worktree_path.to_string_lossy().into_owned(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            created_at: now,
            updated_at: now,
        };
        self.streams.upsert(&stream).await?;
        info!(stream_id = %stream.id, slug, "worktree stream created");
        Ok(stream)
    }

    /// List all streams ordered with primary first.
    pub async fn list_streams(&self) -> Result<Vec<Stream>, SessionError> {
        Ok(self.streams.list().await?)
    }

    /// Get the runtime-state-pinned current stream (None if nothing
    /// has been selected yet — caller typically falls back to primary).
    pub async fn current(&self) -> Result<Option<Stream>, SessionError> {
        match self.streams.current_id().await? {
            None => Ok(None),
            Some(id) => Ok(self.streams.get(&id).await?),
        }
    }

    /// Set the current stream pointer. `None` clears the pointer.
    pub async fn set_current(&self, id: Option<&StreamId>) -> Result<(), SessionError> {
        if let Some(id) = id {
            // Validate the target exists before writing the pointer.
            self.streams
                .get(id)
                .await?
                .ok_or(SessionError::Storage(DomainError::NotFound))?;
        }
        self.streams.set_current(id).await?;
        Ok(())
    }

    pub async fn rename(
        &self,
        id: &StreamId,
        title: impl Into<String>,
    ) -> Result<Stream, SessionError> {
        let mut s = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        s.title = title.into();
        s.updated_at = Timestamp::now();
        self.streams.upsert(&s).await?;
        Ok(s)
    }

    /// Set per-stream pane targets. Either field can be empty to clear.
    pub async fn set_panes(
        &self,
        id: &StreamId,
        working: Option<String>,
        talking: Option<String>,
    ) -> Result<Stream, SessionError> {
        let mut s = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        if let Some(w) = working {
            s.working_pane = w;
        }
        if let Some(t) = talking {
            s.talking_pane = t;
        }
        s.updated_at = Timestamp::now();
        self.streams.upsert(&s).await?;
        Ok(s)
    }

    /// Update the stream's recorded branch + branch_ref to reflect a
    /// successful checkout. Doesn't itself run the checkout — the
    /// caller (typically the git-ops layer) does that and then asks
    /// us to record the new state.
    pub async fn record_branch_checkout(
        &self,
        id: &StreamId,
        branch: impl Into<String>,
    ) -> Result<Stream, SessionError> {
        let mut s = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        let branch = branch.into();
        s.branch = branch.clone();
        s.branch_ref = format!("refs/heads/{branch}");
        s.updated_at = Timestamp::now();
        self.streams.upsert(&s).await?;
        Ok(s)
    }

    /// Delete a stream. The primary cannot be deleted — that's the
    /// project itself and would leave the workspace in an incoherent
    /// state.
    pub async fn delete_stream(&self, id: &StreamId) -> Result<(), SessionError> {
        let stream = self
            .streams
            .get(id)
            .await?
            .ok_or(SessionError::Storage(DomainError::NotFound))?;
        if stream.kind == StreamKind::Primary {
            return Err(SessionError::Storage(DomainError::Invariant(
                "cannot delete primary stream".into(),
            )));
        }
        // For worktree streams, tear down the on-disk worktree first
        // (best-effort — if `git worktree remove` fails, the user
        // can still see the row in the DB and clean up manually).
        let path = Path::new(&stream.worktree_path);
        if path.exists() {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&self.layout.project_dir)
                .arg("worktree")
                .arg("remove")
                .arg("--force")
                .arg(path)
                .output();
        }
        self.streams.delete(id).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore};
    use std::process::Command;
    use tempfile::tempdir;

    fn init_repo(dir: &Path) {
        let repo = git2::Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        config.set_str("init.defaultBranch", "main").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    fn make_service() -> (StreamService, tempfile::TempDir) {
        let project = tempdir().unwrap();
        init_repo(project.path());
        let layout = WorkspaceLayout::for_project(project.path());
        let db = Database::in_memory();
        let streams = Arc::new(SqliteStreamStore::new(db));
        let service = StreamService::new(layout, streams);
        (service, project)
    }

    #[tokio::test]
    async fn validate_workspace_passes_for_repo() {
        let (svc, _dir) = make_service();
        svc.validate_workspace().unwrap();
    }

    #[tokio::test]
    async fn validate_rejects_non_repo() {
        let project = tempdir().unwrap();
        let layout = WorkspaceLayout::for_project(project.path());
        let db = Database::in_memory();
        let streams = Arc::new(SqliteStreamStore::new(db));
        let svc = StreamService::new(layout, streams);
        let err = svc.validate_workspace().unwrap_err();
        assert!(matches!(err, SessionError::NotARepo(_)));
    }

    #[tokio::test]
    async fn ensure_primary_creates_then_idempotent() {
        let (svc, _dir) = make_service();
        let first = svc.ensure_primary().await.unwrap();
        assert_eq!(first.kind, StreamKind::Primary);
        let second = svc.ensure_primary().await.unwrap();
        assert_eq!(first.id, second.id);
    }

    #[tokio::test]
    async fn create_worktree_makes_worktree_and_row() {
        let (svc, dir) = make_service();
        let _primary = svc.ensure_primary().await.unwrap();
        let stream = svc
            .create_worktree("feature-1", "Feature 1", "feature-1", "main")
            .await
            .unwrap();
        assert_eq!(stream.kind, StreamKind::Worktree);
        let path = dir.path().join(".oxplow/worktrees/feature-1");
        assert!(path.exists(), "worktree dir should exist at {path:?}");
    }

    #[tokio::test]
    async fn create_worktree_requires_primary() {
        let (svc, _dir) = make_service();
        let err = svc
            .create_worktree("feature-1", "Feature 1", "feature-1", "main")
            .await
            .unwrap_err();
        assert!(matches!(err, SessionError::PrimaryMissing));
    }

    #[tokio::test]
    async fn delete_primary_rejected() {
        let (svc, _dir) = make_service();
        let primary = svc.ensure_primary().await.unwrap();
        let err = svc.delete_stream(&primary.id).await.unwrap_err();
        assert!(matches!(
            err,
            SessionError::Storage(DomainError::Invariant(_))
        ));
    }

    #[tokio::test]
    async fn delete_worktree_removes_row_and_dir() {
        let (svc, dir) = make_service();
        svc.ensure_primary().await.unwrap();
        let stream = svc
            .create_worktree("feature-rm", "Feature", "feature-rm", "main")
            .await
            .unwrap();
        let path = dir.path().join(".oxplow/worktrees/feature-rm");
        assert!(path.exists());
        svc.delete_stream(&stream.id).await.unwrap();
        // git worktree remove deletes the dir; the row is gone.
        assert!(svc.list_streams().await.unwrap().iter().all(|s| s.id != stream.id));
        // best-effort dir removal — verify
        assert!(!path.exists(), "worktree dir should be removed");
    }

    #[tokio::test]
    async fn list_orders_primary_first() {
        let (svc, _dir) = make_service();
        svc.ensure_primary().await.unwrap();
        svc.create_worktree("a", "A", "a", "main").await.unwrap();
        svc.create_worktree("b", "B", "b", "main").await.unwrap();
        let list = svc.list_streams().await.unwrap();
        assert_eq!(list[0].kind, StreamKind::Primary);
        assert!(list[1..].iter().all(|s| s.kind == StreamKind::Worktree));
    }

    /// Suppress unused warning when the test for `Command` is gone.
    #[allow(dead_code)]
    fn _silence(_: Command) {}
}

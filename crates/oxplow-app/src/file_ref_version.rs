//! Capture-time helper that resolves the `(local_snapshot_id,
//! closest_git_version, git_version_exact)` triple stamped on every
//! file reference. Called by the effort-file recorder and the wiki
//! ref sync so the agent doesn't have to think about it.
//!
//! Resolution policy (matches V20 docs):
//!   * `local_snapshot_id` is the snapshot the caller already pinned
//!     the reference to (effort end snapshot, current wiki snapshot,
//!     etc.).
//!   * If that snapshot row has `git_commit` set (clean worktree at
//!     capture or re-stamped later), use it with
//!     `git_version_exact = true`.
//!   * Otherwise read HEAD via `oxplow_git::head_commit_sha` and
//!     stamp it with `git_version_exact = false`. The cascade in
//!     `set_snapshot_git_commit` will flip exact -> true if and when
//!     the snapshot itself gets a commit attached.
//!   * If neither is available (no commit yet, headless repo) the
//!     `closest_git_version` stays `None`.

use std::path::Path;
use std::sync::Arc;

use oxplow_db::SqliteSnapshotStore;
use oxplow_domain::DomainError;

/// Resolved version triple ready to stamp onto a file ref. Owned
/// (no borrows) so callers can pass it across `spawn_blocking`
/// boundaries without lifetime gymnastics.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFileVersion {
    pub local_snapshot_id: i64,
    pub closest_git_version: Option<String>,
    pub git_version_exact: bool,
}

impl ResolvedFileVersion {
    /// Borrowed view suitable for [`oxplow_db::EffortStore::record_file`].
    pub fn as_ref(&self) -> oxplow_db::FileRefVersion<'_> {
        oxplow_db::FileRefVersion {
            local_snapshot_id: self.local_snapshot_id,
            closest_git_version: self.closest_git_version.as_deref(),
            git_version_exact: self.git_version_exact,
        }
    }
}

pub async fn resolve(
    snapshot_store: &Arc<SqliteSnapshotStore>,
    project_dir: &Path,
    local_snapshot_id: i64,
) -> Result<ResolvedFileVersion, DomainError> {
    if let Some(sha) = snapshot_store
        .get_snapshot_git_commit(local_snapshot_id)
        .await?
    {
        return Ok(ResolvedFileVersion {
            local_snapshot_id,
            closest_git_version: Some(sha),
            git_version_exact: true,
        });
    }
    let project_dir = project_dir.to_path_buf();
    let head = tokio::task::spawn_blocking(move || oxplow_git::head_commit_sha(&project_dir))
        .await
        .unwrap_or(None);
    Ok(ResolvedFileVersion {
        local_snapshot_id,
        closest_git_version: head,
        git_version_exact: false,
    })
}

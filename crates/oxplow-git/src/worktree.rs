use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum EnsureWorktreeError {
    #[error("not a git repo: {0}")]
    NotRepo(PathBuf),
    #[error("git worktree add failed: {0}")]
    WorktreeAdd(String),
}

/// Ensure that a worktree exists at `worktree_path` checked out to
/// `branch`, creating the branch from `branch_source` if needed.
///
/// Idempotent: if the worktree already exists, this is a no-op.
/// Uses `git worktree add` shell-out because libgit2's worktree
/// support is incomplete for the cases we care about (e.g.
/// `--no-track` flags, `-B` to force re-create).
pub fn ensure_worktree(
    repo_root: impl AsRef<Path>,
    worktree_path: impl AsRef<Path>,
    branch: &str,
    branch_source: &str,
) -> Result<(), EnsureWorktreeError> {
    let repo_root = repo_root.as_ref();
    let worktree_path = worktree_path.as_ref();

    if !crate::repo::is_git_repo(repo_root) {
        return Err(EnsureWorktreeError::NotRepo(repo_root.to_path_buf()));
    }

    if worktree_path.exists() {
        info!(
            path = %worktree_path.display(),
            "worktree already exists; skipping ensure"
        );
        return Ok(());
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            EnsureWorktreeError::WorktreeAdd(format!(
                "create worktree parent dir {}: {e}",
                parent.display()
            ))
        })?;
    }

    // Create the branch from source if it doesn't exist; otherwise
    // just attach.
    let branch_exists = {
        let repo = git2::Repository::open(repo_root)
            .map_err(|e| EnsureWorktreeError::WorktreeAdd(e.message().to_string()))?;
        let exists = repo.find_branch(branch, git2::BranchType::Local).is_ok();
        exists
    };

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_root).arg("worktree").arg("add");
    if !branch_exists {
        cmd.arg("-b").arg(branch).arg(worktree_path).arg(branch_source);
    } else {
        cmd.arg(worktree_path).arg(branch);
    }

    let out = cmd
        .output()
        .map_err(|e| EnsureWorktreeError::WorktreeAdd(format!("spawn: {e}")))?;
    if !out.status.success() {
        return Err(EnsureWorktreeError::WorktreeAdd(format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    info!(
        path = %worktree_path.display(),
        branch,
        "worktree created"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        // Set init.defaultBranch so the branch source matches.
        config.set_str("init.defaultBranch", "main").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        dir
    }

    fn current_branch(repo: &git2::Repository) -> String {
        repo.head()
            .unwrap()
            .shorthand()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "main".into())
    }

    #[test]
    fn creates_new_worktree_with_new_branch() {
        let repo_dir = init_repo();
        let repo = git2::Repository::open(repo_dir.path()).unwrap();
        let main = current_branch(&repo);

        let wt_root = tempdir().unwrap();
        let wt_path = wt_root.path().join("feature-wt");
        let result = ensure_worktree(repo_dir.path(), &wt_path, "feature", &main);
        assert!(result.is_ok(), "{result:?}");
        assert!(wt_path.exists());
        // The new branch should now exist in the main repo.
        assert!(repo.find_branch("feature", git2::BranchType::Local).is_ok());
    }

    #[test]
    fn idempotent_when_path_exists() {
        let repo_dir = init_repo();
        let repo = git2::Repository::open(repo_dir.path()).unwrap();
        let main = current_branch(&repo);

        let wt_root = tempdir().unwrap();
        let wt_path = wt_root.path().join("dup");
        ensure_worktree(repo_dir.path(), &wt_path, "feature", &main).unwrap();
        // Second call should be a no-op (and therefore Ok), even
        // though `git worktree add` itself would fail loudly.
        ensure_worktree(repo_dir.path(), &wt_path, "feature", &main).unwrap();
    }

    #[test]
    fn rejects_non_repo() {
        let dir = tempdir().unwrap();
        let result = ensure_worktree(dir.path(), dir.path().join("wt"), "x", "y");
        assert!(matches!(result, Err(EnsureWorktreeError::NotRepo(_))));
    }
}

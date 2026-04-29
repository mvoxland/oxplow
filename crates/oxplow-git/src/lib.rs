//! Git integration for oxplow.
//!
//! Repo detection, branch listing, worktree management, and conflict
//! state. Uses `git2` for in-process ops where it's well-supported,
//! falling back to `Command::new("git")` for cases libgit2 doesn't
//! cover well (e.g. `git worktree add`).

mod branch;
mod conflict;
mod repo;
mod worktree;

pub use branch::{list_branches, BranchRef, BranchRefKind};
pub use conflict::{get_repo_conflict_state, GitOperationKind, RepoConflictState};
pub use repo::{detect_current_branch, is_git_repo, is_git_worktree};
pub use worktree::{ensure_worktree, EnsureWorktreeError};

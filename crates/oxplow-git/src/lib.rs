//! Git integration for oxplow.
//!
//! Refs watching, blame, worktree add/remove, branch checkout, status.
//! In-process ops use `git2`; a few worktree ops fall back to
//! `Command::new("git")` where libgit2 doesn't cover the use case.

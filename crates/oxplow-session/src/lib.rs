//! Stream + worktree lifecycle.
//!
//! Encodes the primary-vs-worktree invariant from
//! `.context/architecture.md`: exactly one primary stream per project,
//! everything else is a worktree under `.oxplow/worktrees/<slug>/`.

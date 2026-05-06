//! `git status`-like inspection via libgit2. Returns a path → status
//! map shaped to feed `workspace::list_workspace_entries`.

use std::collections::HashMap;
use std::path::Path;

use crate::workspace::GitFileStatus;

/// Map every changed/untracked path under `repo_path` to its
/// classification. Fast-path: empty map if not a git repo.
pub fn list_git_statuses(repo_path: &Path) -> HashMap<String, GitFileStatus> {
    let mut out = HashMap::new();
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return out;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .include_ignored(false)
        .include_unmodified(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return out;
    };
    for entry in statuses.iter() {
        let Some(path) = entry.path().map(|s| s.to_string()) else {
            continue;
        };
        let s = entry.status();
        // Classify in priority order matching the TS contract.
        // Untracked vs Added: Untracked = worktree-only-new (no index
        // entry). Added = staged-new (INDEX_NEW set). Modified covers
        // both index-modified and worktree-modified cases. Deleted
        // and Renamed surface their own flags.
        let classification = if s.contains(git2::Status::INDEX_NEW) {
            GitFileStatus::Added
        } else if s.contains(git2::Status::WT_NEW) {
            GitFileStatus::Untracked
        } else if s.contains(git2::Status::WT_DELETED) || s.contains(git2::Status::INDEX_DELETED) {
            GitFileStatus::Deleted
        } else if s.contains(git2::Status::WT_RENAMED) || s.contains(git2::Status::INDEX_RENAMED) {
            GitFileStatus::Renamed
        } else if s.contains(git2::Status::WT_MODIFIED) || s.contains(git2::Status::INDEX_MODIFIED)
        {
            GitFileStatus::Modified
        } else {
            continue;
        };
        out.insert(path, classification);
    }
    out
}

/// Single-path status lookup. Convenience wrapper around the bulk
/// call; not optimized for hot-path use.
pub fn status_for_path(repo_path: &Path, target: &str) -> Option<GitFileStatus> {
    list_git_statuses(repo_path).get(target).copied()
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
        config.set_str("user.email", "t@example.com").unwrap();
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

    #[test]
    fn untracked_file_classified_as_untracked() {
        let dir = init_repo();
        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        let s = list_git_statuses(dir.path());
        assert_eq!(s.get("new.txt"), Some(&GitFileStatus::Untracked));
    }

    #[test]
    fn modified_file_classified_as_modified() {
        let dir = init_repo();
        let repo = git2::Repository::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.txt"), "v1").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("a.txt")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add a", &tree, &[&head])
            .unwrap();
        // Refresh the in-memory index from disk so the post-commit
        // state matches the file system before we compare.
        let mut idx = repo.index().unwrap();
        idx.read(true).unwrap();
        std::fs::write(dir.path().join("a.txt"), "v2").unwrap();
        let s = list_git_statuses(dir.path());
        assert_eq!(s.get("a.txt"), Some(&GitFileStatus::Modified));
    }
}

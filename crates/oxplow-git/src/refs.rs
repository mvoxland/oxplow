//! Refs + file-at-ref reads.
//!
//! Powers ref-pickers (branch / tag / remote dropdowns), file-history
//! views, and the diff editor's "open this file at HEAD/<sha>" path.

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::log::GitLogCommit;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RefOption {
    pub label: String,
    pub r#ref: String,
    pub kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Local,
    Remote,
    Tag,
    Head,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GroupedGitRefs {
    pub locals: Vec<RefOption>,
    pub remotes: Vec<RefOption>,
    pub tags: Vec<RefOption>,
}

/// Returns every ref grouped into local branches, remote branches and
/// tags. Omits the `HEAD` symbolic ref since it's never useful as a
/// picker option.
pub fn list_all_refs(repo_path: &Path) -> GroupedGitRefs {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => {
            return GroupedGitRefs {
                locals: vec![],
                remotes: vec![],
                tags: vec![],
            }
        }
    };
    let mut locals = vec![];
    let mut remotes = vec![];
    let mut tags = vec![];
    if let Ok(refs) = repo.references() {
        for r in refs.flatten() {
            let name = match r.name() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name == "HEAD" {
                continue;
            }
            if let Some(short) = name.strip_prefix("refs/heads/") {
                locals.push(RefOption {
                    label: short.to_string(),
                    r#ref: name.clone(),
                    kind: RefKind::Local,
                });
            } else if let Some(short) = name.strip_prefix("refs/remotes/") {
                if short == "origin/HEAD" {
                    continue;
                }
                remotes.push(RefOption {
                    label: short.to_string(),
                    r#ref: name.clone(),
                    kind: RefKind::Remote,
                });
            } else if let Some(short) = name.strip_prefix("refs/tags/") {
                tags.push(RefOption {
                    label: short.to_string(),
                    r#ref: name.clone(),
                    kind: RefKind::Tag,
                });
            }
        }
    }
    locals.sort_by(|a, b| a.label.cmp(&b.label));
    remotes.sort_by(|a, b| a.label.cmp(&b.label));
    tags.sort_by(|a, b| b.label.cmp(&a.label));
    GroupedGitRefs {
        locals,
        remotes,
        tags,
    }
}

/// Read a file's contents at a given ref. Returns `None` if the path
/// does not exist at that ref.
pub fn read_file_at_ref(repo_path: &Path, r#ref: &str, path: &str) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let obj = repo.revparse_single(r#ref).ok()?;
    let commit = obj.peel_to_commit().ok()?;
    let tree = commit.tree().ok()?;
    let entry = tree.get_path(Path::new(path)).ok()?;
    let blob = entry.to_object(&repo).ok()?;
    let blob = blob.as_blob()?;
    String::from_utf8(blob.content().to_vec()).ok()
}

/// List the most recent commits that touched `path`, up to `limit`.
pub fn list_file_commits(repo_path: &Path, path: &str, limit: usize) -> Vec<GitLogCommit> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut walker = match repo.revwalk() {
        Ok(w) => w,
        Err(_) => return vec![],
    };
    if walker.push_head().is_err() {
        return vec![];
    }
    walker.set_sorting(git2::Sort::TIME).ok();

    let target = Path::new(path);
    let mut out = Vec::new();
    for oid_res in walker {
        let oid = match oid_res {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Compare with first parent (or empty tree if no parents).
        let touches = touches_path(&repo, &commit, target).unwrap_or(false);
        if !touches {
            continue;
        }
        let sha = commit.id().to_string();
        let short_sha = sha[..7.min(sha.len())].to_string();
        out.push(GitLogCommit {
            sha,
            short_sha,
            author: commit.author().name().unwrap_or("").to_string(),
            email: commit.author().email().unwrap_or("").to_string(),
            timestamp_secs: commit.time().seconds(),
            subject: commit.summary().unwrap_or("").to_string(),
            parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn touches_path(
    repo: &git2::Repository,
    commit: &git2::Commit<'_>,
    path: &Path,
) -> Result<bool, git2::Error> {
    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };
    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.pathspec(path);
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut diff_opts))?;
    Ok(diff.deltas().any(|_| true))
}

/// List recent remote branches sorted by author commit time. Returns
/// up to `limit` entries.
pub fn list_recent_remote_branches(repo_path: &Path, limit: usize) -> Vec<RemoteBranchEntry> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut entries = Vec::new();
    if let Ok(refs) = repo.references_glob("refs/remotes/*/*") {
        for r in refs.flatten() {
            let name = match r.shorthand() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name.ends_with("/HEAD") {
                continue;
            }
            let commit = match r.peel_to_commit() {
                Ok(c) => c,
                Err(_) => continue,
            };
            entries.push(RemoteBranchEntry {
                ref_name: r.name().unwrap_or("").to_string(),
                short_name: name,
                last_commit_at: commit.time().seconds(),
                last_commit_subject: commit.summary().unwrap_or("").to_string(),
            });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.last_commit_at));
    entries.truncate(limit);
    entries
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RemoteBranchEntry {
    pub ref_name: String,
    pub short_name: String,
    pub last_commit_at: i64,
    pub last_commit_subject: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn init_with_commit(dir: &Path, file: &str, content: &str) {
        Command::new("git")
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "t@e.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "t"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::fs::write(dir.join(file), content).unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "msg"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn list_all_refs_returns_local_main() {
        let dir = tempdir().unwrap();
        init_with_commit(dir.path(), "a.txt", "x");
        let refs = list_all_refs(dir.path());
        assert!(refs.locals.iter().any(|r| r.label == "main"));
    }

    #[test]
    fn read_file_at_head_returns_content() {
        let dir = tempdir().unwrap();
        init_with_commit(dir.path(), "a.txt", "hello");
        let body = read_file_at_ref(dir.path(), "HEAD", "a.txt").unwrap();
        assert_eq!(body, "hello");
    }

    #[test]
    fn list_file_commits_finds_commit_touching_path() {
        let dir = tempdir().unwrap();
        init_with_commit(dir.path(), "a.txt", "hello");
        let commits = list_file_commits(dir.path(), "a.txt", 10);
        assert_eq!(commits.len(), 1);
    }
}

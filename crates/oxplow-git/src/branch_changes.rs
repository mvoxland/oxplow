//! Branch-changes diff: what's different between HEAD and a base
//! ref, including any working-tree wip edits on top.
//!
//! Mirrors the original TS surface: returns a flat list of files
//! with adds/dels counts, and includes untracked files from
//! `status --porcelain`.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BranchChangeEntry {
    pub path: String,
    pub original_path: Option<String>,
    pub change: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BranchChanges {
    pub base_ref: String,
    pub merge_base: Option<String>,
    pub files: Vec<BranchChangeEntry>,
}

fn run_capturing(args: &[&str], cwd: &Path) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(cwd).output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn list_branch_changes(repo: &Path, base_ref: &str) -> BranchChanges {
    if !crate::repo::is_git_repo(repo) {
        return BranchChanges {
            base_ref: base_ref.to_string(),
            merge_base: None,
            files: vec![],
        };
    }
    let merge_base = run_capturing(&["merge-base", "HEAD", base_ref], repo)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let merge_base_str = match &merge_base {
        Some(s) => s.clone(),
        None => {
            return BranchChanges {
                base_ref: base_ref.to_string(),
                merge_base: None,
                files: vec![],
            }
        }
    };

    let name_status = run_capturing(
        &["diff", "--name-status", "-z", &merge_base_str],
        repo,
    )
    .unwrap_or_default();
    let numstat = run_capturing(&["diff", "--numstat", "-z", &merge_base_str], repo).unwrap_or_default();

    let mut entries = parse_name_status_z(&name_status);
    let counts = parse_numstat_z(&numstat);

    for entry in entries.iter_mut() {
        if let Some((adds, dels)) = counts.iter().find_map(|c| {
            if c.0 == entry.path {
                Some((c.1, c.2))
            } else {
                None
            }
        }) {
            entry.additions = adds;
            entry.deletions = dels;
        }
    }

    // Untracked files via status --porcelain
    if let Some(status) =
        run_capturing(&["status", "--porcelain", "--untracked-files=all"], repo)
    {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("?? ") {
                if !entries.iter().any(|e| e.path == rest) {
                    entries.push(BranchChangeEntry {
                        path: rest.to_string(),
                        original_path: None,
                        change: ChangeKind::Untracked,
                        additions: 0,
                        deletions: 0,
                    });
                }
            }
        }
    }

    BranchChanges {
        base_ref: base_ref.to_string(),
        merge_base,
        files: entries,
    }
}

fn parse_name_status_z(raw: &str) -> Vec<BranchChangeEntry> {
    let mut out = Vec::new();
    let mut iter = raw.split('\0').peekable();
    while let Some(status) = iter.next() {
        if status.is_empty() {
            continue;
        }
        let kind_char = status.chars().next().unwrap_or(' ');
        match kind_char {
            'A' | 'M' | 'D' | 'T' => {
                let path = match iter.next() {
                    Some(p) if !p.is_empty() => p,
                    _ => continue,
                };
                out.push(BranchChangeEntry {
                    path: path.to_string(),
                    original_path: None,
                    change: match kind_char {
                        'A' => ChangeKind::Added,
                        'D' => ChangeKind::Deleted,
                        _ => ChangeKind::Modified,
                    },
                    additions: 0,
                    deletions: 0,
                });
            }
            'R' | 'C' => {
                let from = iter.next().unwrap_or("").to_string();
                let to = iter.next().unwrap_or("").to_string();
                if from.is_empty() || to.is_empty() {
                    continue;
                }
                out.push(BranchChangeEntry {
                    path: to,
                    original_path: Some(from),
                    change: if kind_char == 'R' {
                        ChangeKind::Renamed
                    } else {
                        ChangeKind::Copied
                    },
                    additions: 0,
                    deletions: 0,
                });
            }
            _ => {}
        }
    }
    out
}

fn parse_numstat_z(raw: &str) -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    let mut iter = raw.split('\0').peekable();
    while let Some(line) = iter.next() {
        if line.is_empty() {
            continue;
        }
        // Format: "<adds>\t<dels>\t<path>" or for renames "<adds>\t<dels>\t" then two NUL-separated paths.
        let mut parts = line.splitn(3, '\t');
        let adds: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let dels: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let path = parts.next().unwrap_or("").to_string();
        if path.is_empty() {
            // Rename: next two NUL-separated tokens are old/new paths.
            let _old = iter.next();
            let new_path = iter.next().unwrap_or("").to_string();
            if !new_path.is_empty() {
                out.push((new_path, adds, dels));
            }
        } else {
            out.push((path, adds, dels));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;
    use tempfile::tempdir;

    fn init_repo(dir: &Path) {
        Cmd::new("git").args(["init", "-q", "--initial-branch=main"]).current_dir(dir).output().unwrap();
        Cmd::new("git").args(["config", "user.email", "t@e.com"]).current_dir(dir).output().unwrap();
        Cmd::new("git").args(["config", "user.name", "t"]).current_dir(dir).output().unwrap();
    }

    fn commit(dir: &Path, msg: &str) {
        Cmd::new("git").args(["add", "-A"]).current_dir(dir).output().unwrap();
        Cmd::new("git").args(["commit", "-m", msg]).current_dir(dir).output().unwrap();
    }

    #[test]
    fn detects_added_file_against_base() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        commit(dir.path(), "init");
        // Create a feature branch and add a file.
        Cmd::new("git").args(["checkout", "-b", "feat"]).current_dir(dir.path()).output().unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        commit(dir.path(), "add b");
        let changes = list_branch_changes(dir.path(), "main");
        assert!(changes.files.iter().any(|f| f.path == "b.txt" && f.change == ChangeKind::Added));
    }

    #[test]
    fn detects_untracked_file() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        commit(dir.path(), "init");
        std::fs::write(dir.path().join("u.txt"), "u").unwrap();
        let changes = list_branch_changes(dir.path(), "main");
        assert!(changes
            .files
            .iter()
            .any(|f| f.path == "u.txt" && f.change == ChangeKind::Untracked));
    }
}

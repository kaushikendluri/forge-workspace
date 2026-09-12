//! Git operations: repository discovery and status/branch inspection for
//! the Projects and Changes tabs (worktree management for agent isolation
//! and diff generation land in M3). Shells out to the system `git` binary
//! (resolved via `os_adapter::OperatingSystemAdapter::resolve_executable`)
//! rather than linking libgit2, to stay compatible with the user's own git
//! config/credentials/hooks.
//!
//! Behind a `GitService` trait (rather than free functions) so
//! `AppState` can hold a `Box<dyn GitService>` and commands don't need to
//! know they're shelling out at all — useful for testing later without a
//! real git binary.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::os_adapter::OperatingSystemAdapter;

/// One entry in `GitStatus::staged` / `GitStatus::unstaged` — a single-letter
/// porcelain v2 status code (`M`, `A`, `D`, `R`, `C`, `U`, ...) for one side
/// (index or worktree) of a tracked file's change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStatusEntry {
    pub path: String,
    pub status_code: String,
}

/// Working-tree status for a repository, parsed from
/// `git status --porcelain=v2 --branch -z`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub current_branch: Option<String>,
    pub staged: Vec<FileStatusEntry>,
    pub unstaged: Vec<FileStatusEntry>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct GitDiff {
    pub path: String,
    pub original: String,
    pub modified: String,
}

/// Git operations needed by `commands::project_commands` and
/// `commands::git_commands`. Implemented for real by `GitCliService`;
/// having this as a trait (rather than free functions) lets `AppState`
/// hold it as `Box<dyn GitService>` and keeps the door open for a fake
/// implementation in tests.
pub trait GitService: Send + Sync {
    /// Working-tree status (branch + staged/unstaged/untracked files).
    fn status(&self, repo_path: &Path) -> AppResult<GitStatus>;

    /// All local branches, with the checked-out one flagged.
    fn branches(&self, repo_path: &Path) -> AppResult<Vec<BranchInfo>>;

    /// The currently checked-out branch, or `None` if HEAD is detached.
    fn current_branch(&self, repo_path: &Path) -> AppResult<Option<String>>;

    /// True if `path` is (the root of, or inside) a git working tree.
    fn is_git_repository(&self, path: &Path) -> bool;

    /// Runs `git init` in `path`, which must already exist.
    fn init(&self, path: &Path) -> AppResult<()>;

    /// Creates a new worktree at `worktree_path` on a new branch
    /// `branch_name`, for isolating one agent's changes from the primary
    /// checkout. Default body panics — implemented in M3.
    fn add_worktree(
        &self,
        _repo_root: &Path,
        _worktree_path: &Path,
        _branch_name: &str,
        _base_branch: &str,
    ) -> AppResult<()> {
        todo!("M3: git worktree add -b <branch_name> <worktree_path> <base_branch>")
    }

    /// Removes a worktree previously created by `add_worktree`. Default
    /// body panics — implemented in M3.
    fn remove_worktree(&self, _repo_root: &Path, _worktree_path: &Path) -> AppResult<()> {
        todo!("M3: git worktree remove <worktree_path>")
    }

    /// Diff for a single file between two refs (or working tree vs. HEAD).
    /// Default body panics — implemented alongside the diff viewer.
    fn diff_file(&self, _repo_root: &Path, _path: &str, _base_ref: &str) -> AppResult<GitDiff> {
        todo!("M2/M3: git show <base_ref>:<path> + read working tree file")
    }
}

/// `GitService` backed by shelling out to the system `git` binary.
pub struct GitCliService {
    git_path: PathBuf,
}

impl GitCliService {
    /// Resolves `git` once via the OS adapter's PATH search so every call
    /// doesn't repeat the lookup; falls back to the bare name `"git"` (let
    /// the OS's own PATH search/`Command` error surface the real problem)
    /// if it isn't found up front.
    pub fn new(os_adapter: &dyn OperatingSystemAdapter) -> Self {
        let git_path = os_adapter
            .resolve_executable("git")
            .unwrap_or_else(|| PathBuf::from("git"));
        Self { git_path }
    }

    fn run(&self, repo_path: &Path, args: &[&str]) -> AppResult<Vec<u8>> {
        let output = Command::new(&self.git_path)
            .args(args)
            .current_dir(repo_path)
            .output()
            .map_err(|e| {
                AppError::Other(format!(
                    "failed to run '{} {}' ({}): {e}",
                    self.git_path.display(),
                    args.join(" "),
                    repo_path.display(),
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let message = if stderr.is_empty() {
                format!("git {} exited with {}", args.join(" "), output.status)
            } else {
                stderr
            };
            return Err(AppError::Other(message));
        }

        Ok(output.stdout)
    }
}

impl GitService for GitCliService {
    fn status(&self, repo_path: &Path) -> AppResult<GitStatus> {
        let raw = self.run(repo_path, &["status", "--porcelain=v2", "--branch", "-z"])?;
        Ok(parse_status(&String::from_utf8_lossy(&raw)))
    }

    fn branches(&self, repo_path: &Path) -> AppResult<Vec<BranchInfo>> {
        let raw = self.run(repo_path, &["branch", "--format=%(refname:short)|%(HEAD)"])?;
        let text = String::from_utf8_lossy(&raw);
        let branches = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let mut parts = line.splitn(2, '|');
                let name = parts.next().unwrap_or("").to_string();
                let is_current = parts.next().unwrap_or("").trim() == "*";
                BranchInfo { name, is_current }
            })
            .collect();
        Ok(branches)
    }

    fn current_branch(&self, repo_path: &Path) -> AppResult<Option<String>> {
        let raw = self.run(repo_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        let name = String::from_utf8_lossy(&raw).trim().to_string();
        if name.is_empty() || name == "HEAD" {
            // Empty output shouldn't happen; "HEAD" means a detached HEAD.
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    fn is_git_repository(&self, path: &Path) -> bool {
        if path.join(".git").exists() {
            return true;
        }
        // Handles worktrees/bare setups where `.git` isn't a plain directory
        // (or is missing entirely but `path` is still inside a work tree).
        self.run(path, &["rev-parse", "--is-inside-work-tree"])
            .map(|out| String::from_utf8_lossy(&out).trim() == "true")
            .unwrap_or(false)
    }

    fn init(&self, path: &Path) -> AppResult<()> {
        self.run(path, &["init"]).map(|_| ())
    }
}

/// Parses `git status --porcelain=v2 --branch -z` output. Records are
/// NUL-terminated instead of newline-terminated with `-z`, and rename/copy
/// records (`2 ...`) are followed by an extra NUL-terminated token holding
/// the original path — so this walks an iterator over NUL-split tokens
/// rather than `.lines()`.
fn parse_status(raw: &str) -> GitStatus {
    let mut status = GitStatus::default();
    let mut tokens = raw.split('\0').filter(|t| !t.is_empty());

    while let Some(token) = tokens.next() {
        if let Some(rest) = token.strip_prefix("# branch.head ") {
            status.current_branch = if rest == "(detached)" {
                None
            } else {
                Some(rest.to_string())
            };
        } else if let Some(path) = token.strip_prefix("? ") {
            status.untracked.push(path.to_string());
        } else if let Some(rest) = token.strip_prefix("1 ") {
            // "1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>"
            let fields: Vec<&str> = rest.splitn(8, ' ').collect();
            if fields.len() == 8 {
                push_status_entry(&mut status, fields[0], fields[7].to_string());
            }
        } else if let Some(rest) = token.strip_prefix("2 ") {
            // "2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>" then
            // a separate NUL-terminated token with the original path.
            let fields: Vec<&str> = rest.splitn(9, ' ').collect();
            let orig_path = tokens.next().unwrap_or("");
            if fields.len() == 9 {
                let display = if orig_path.is_empty() {
                    fields[8].to_string()
                } else {
                    format!("{} <- {}", fields[8], orig_path)
                };
                push_status_entry(&mut status, fields[0], display);
            }
        } else if let Some(rest) = token.strip_prefix("u ") {
            // "u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>"
            let fields: Vec<&str> = rest.splitn(10, ' ').collect();
            if fields.len() == 10 {
                status.unstaged.push(FileStatusEntry {
                    path: fields[9].to_string(),
                    status_code: fields[0].to_string(),
                });
            }
        }
        // Other "#" headers (branch.oid, branch.upstream, branch.ab) and "!"
        // ignored-file entries aren't needed for this milestone.
    }

    status
}

/// Splits a two-character `XY` porcelain status code into its index (X) and
/// worktree (Y) halves, pushing a `FileStatusEntry` into `staged` and/or
/// `unstaged` for whichever half is non-`.` (a file can be both, e.g. staged
/// then further modified — that's two separate entries, correctly).
fn push_status_entry(status: &mut GitStatus, xy: &str, path: String) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    if x != '.' {
        status.staged.push(FileStatusEntry {
            path: path.clone(),
            status_code: x.to_string(),
        });
    }
    if y != '.' {
        status.unstaged.push(FileStatusEntry {
            path,
            status_code: y.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_and_modified_file() {
        let raw = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -0\01 M. N... 100644 100644 100644 abc def src/lib.rs\0? notes.txt\0";
        let status = parse_status(raw);
        assert_eq!(status.current_branch.as_deref(), Some("main"));
        assert_eq!(status.staged, vec![FileStatusEntry { path: "src/lib.rs".into(), status_code: "M".into() }]);
        assert!(status.unstaged.is_empty());
        assert_eq!(status.untracked, vec!["notes.txt".to_string()]);
    }

    #[test]
    fn detached_head_has_no_branch() {
        let raw = "# branch.head (detached)\0";
        let status = parse_status(raw);
        assert_eq!(status.current_branch, None);
    }

    #[test]
    fn rename_entry_reports_both_paths() {
        let raw = "2 R. N... 100644 100644 100644 abc def R100 new_name.rs\0old_name.rs\0";
        let status = parse_status(raw);
        assert_eq!(status.staged.len(), 1);
        assert_eq!(status.staged[0].path, "new_name.rs <- old_name.rs");
        assert_eq!(status.staged[0].status_code, "R");
    }
}

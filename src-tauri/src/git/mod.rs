//! Git operations: repository discovery, status/branch inspection, and
//! diff/log for the Projects and Changes tabs (worktree management for
//! agent isolation still lands later, alongside agent execution). Shells
//! out to the system `git` binary
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

/// One entry from `git worktree list --porcelain`, for reconciling the
/// `workspaces` DB table against what's actually on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    /// `None` for a detached-HEAD worktree (or a bare repository's own
    /// entry), matching `git worktree list --porcelain`'s `detached`/`bare`
    /// lines carrying no `branch <ref>` line.
    pub branch: Option<String>,
    pub head_sha: String,
}

/// Full before/after file text for the Changes tab's Monaco diff viewer.
/// Built from `git show HEAD:<path>` (original) + the working-tree file
/// (modified) rather than a parsed unified patch — simpler, and Monaco wants
/// full-file text on both sides anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileDiff {
    /// The file's content at `HEAD`, or `""` if it has no committed version
    /// (i.e. it's untracked/newly added).
    pub original: String,
    /// The file's current working-tree content, or `""` if it no longer
    /// exists there (i.e. it was deleted).
    pub modified: String,
    pub is_new_file: bool,
    pub is_deleted: bool,
}

/// One entry from `git log`, for the Changes tab's history view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub email: String,
    /// ISO 8601 author date (`%aI`).
    pub date: String,
    pub subject: String,
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
    /// `branch_name`, based on `base_branch`, for isolating one agent's
    /// changes from the primary checkout. Surfaces git's real stderr on
    /// failure (branch already exists, `worktree_path` already
    /// exists/non-empty, `base_branch` doesn't exist, ...) rather than a
    /// generic message.
    fn add_worktree(
        &self,
        repo_root: &Path,
        worktree_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> AppResult<()>;

    /// Removes a worktree previously created by `add_worktree`. Deliberately
    /// does *not* pass `--force`: if the worktree has uncommitted changes,
    /// git refuses and this lets that error surface honestly rather than
    /// silently discarding an agent's work.
    fn remove_worktree(&self, repo_root: &Path, worktree_path: &Path) -> AppResult<()>;

    /// All worktrees (primary + agent) registered against `repo_root`,
    /// parsed from `git worktree list --porcelain` — used to reconcile the
    /// `workspaces` DB table against what's actually on disk.
    fn list_worktrees(&self, repo_root: &Path) -> AppResult<Vec<WorktreeInfo>>;

    /// Full before/after text for `path`, for the Changes tab's diff viewer.
    /// Handles new files (no `HEAD` version) and deleted files (no
    /// working-tree version) rather than erroring on either.
    fn diff_file(&self, repo_root: &Path, path: &str) -> AppResult<GitFileDiff>;

    /// The `limit` most recent commits reachable from `HEAD`, most recent
    /// first. Returns an empty list (rather than erroring) for a repository
    /// with no commits yet.
    fn log(&self, repo_root: &Path, limit: u32) -> AppResult<Vec<CommitInfo>>;
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

    /// Like `run`, but returns `None` instead of an `Err` on a non-zero
    /// exit — used where a failing git command is an expected, meaningful
    /// outcome (e.g. `git show HEAD:<path>` for a file that doesn't exist at
    /// `HEAD`) rather than a real error to surface.
    fn try_run(&self, repo_path: &Path, args: &[&str]) -> Option<Vec<u8>> {
        let output = Command::new(&self.git_path)
            .args(args)
            .current_dir(repo_path)
            .output()
            .ok()?;
        if output.status.success() {
            Some(output.stdout)
        } else {
            None
        }
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

    fn add_worktree(
        &self,
        repo_root: &Path,
        worktree_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> AppResult<()> {
        let worktree_path_str = worktree_path.to_string_lossy().to_string();
        self.run(
            repo_root,
            &[
                "worktree",
                "add",
                "-b",
                branch_name,
                worktree_path_str.as_str(),
                base_branch,
            ],
        )?;
        Ok(())
    }

    fn remove_worktree(&self, repo_root: &Path, worktree_path: &Path) -> AppResult<()> {
        let worktree_path_str = worktree_path.to_string_lossy().to_string();
        // Intentionally no `--force`: a dirty worktree should make this fail
        // with git's own error, not silently discard the agent's changes.
        self.run(repo_root, &["worktree", "remove", worktree_path_str.as_str()])?;
        Ok(())
    }

    fn list_worktrees(&self, repo_root: &Path) -> AppResult<Vec<WorktreeInfo>> {
        let raw = self.run(repo_root, &["worktree", "list", "--porcelain"])?;
        Ok(parse_worktree_list(&String::from_utf8_lossy(&raw)))
    }

    fn diff_file(&self, repo_root: &Path, path: &str) -> AppResult<GitFileDiff> {
        // `git show HEAD:<path>` fails (no `HEAD`, or the path doesn't exist
        // at `HEAD`) for a new/untracked file — that's expected, not an
        // error, so `try_run` rather than `run`.
        let head_spec = format!("HEAD:{path}");
        let original = self
            .try_run(repo_root, &["show", head_spec.as_str()])
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

        // Similarly, a missing working-tree file just means the file was
        // deleted.
        let modified = std::fs::read(repo_root.join(path))
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

        let is_new_file = original.is_none();
        let is_deleted = modified.is_none();
        if is_new_file && is_deleted {
            return Err(AppError::NotFound(format!(
                "'{path}' exists neither at HEAD nor in the working tree"
            )));
        }

        Ok(GitFileDiff {
            original: original.unwrap_or_default(),
            modified: modified.unwrap_or_default(),
            is_new_file,
            is_deleted,
        })
    }

    fn log(&self, repo_root: &Path, limit: u32) -> AppResult<Vec<CommitInfo>> {
        // Unit-separator (0x1f) between fields, record-separator (0x1e)
        // between commits — neither can appear in git's own output, so no
        // escaping/parsing ambiguity the way there would be with a
        // human-oriented delimiter.
        let format = "%H%x1f%h%x1f%an%x1f%ae%x1f%aI%x1f%s%x1e";
        let limit_str = limit.to_string();
        let pretty_arg = format!("--pretty=format:{format}");
        let args: [&str; 4] = ["log", "-n", limit_str.as_str(), pretty_arg.as_str()];

        // A repository with no commits yet makes `git log` fail — that's a
        // legitimate "no history" state for a brand-new project, not an
        // error worth surfacing, so it's reported as an empty log.
        let Some(raw) = self.try_run(repo_root, &args) else {
            return Ok(Vec::new());
        };
        let text = String::from_utf8_lossy(&raw);

        let commits = text
            .split('\u{1e}')
            .map(str::trim)
            .filter(|record| !record.is_empty())
            .filter_map(|record| {
                let fields: Vec<&str> = record.split('\u{1f}').collect();
                if fields.len() != 6 {
                    return None;
                }
                Some(CommitInfo {
                    sha: fields[0].to_string(),
                    short_sha: fields[1].to_string(),
                    author: fields[2].to_string(),
                    email: fields[3].to_string(),
                    date: fields[4].to_string(),
                    subject: fields[5].to_string(),
                })
            })
            .collect();

        Ok(commits)
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

/// Parses `git worktree list --porcelain` output: one block per worktree
/// (`worktree <path>`, `HEAD <sha>`, then either `branch <ref>` or a bare
/// `detached`/`bare` line), blocks separated by a blank line. A trailing
/// sentinel blank line is appended so the last block is flushed the same way
/// as every other one, whether or not git's own output ends in one.
fn parse_worktree_list(raw: &str) -> Vec<WorktreeInfo> {
    let mut result = Vec::new();
    let mut path: Option<String> = None;
    let mut head_sha: Option<String> = None;
    let mut branch: Option<String> = None;

    for line in raw.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let (Some(path), Some(head_sha)) = (path.take(), head_sha.take()) {
                result.push(WorktreeInfo {
                    path,
                    branch: branch.take(),
                    head_sha,
                });
            }
            branch = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            path = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            head_sha = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(
                rest.strip_prefix("refs/heads/")
                    .unwrap_or(rest)
                    .to_string(),
            );
        }
        // "detached", "bare", "locked"/"locked <reason>", "prunable
        // <reason>" lines carry no data this struct needs.
    }

    result
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

    #[test]
    fn parses_worktree_list_with_branch_and_detached_entries() {
        let raw = "worktree /repo\nHEAD abc123\nbranch refs/heads/main\n\nworktree /repo/.forge-workspace/worktrees/run1\nHEAD def456\nbranch refs/heads/forge/agent/tester/run1\n\nworktree /repo-detached\nHEAD 789abc\ndetached\n";
        let worktrees = parse_worktree_list(raw);
        assert_eq!(worktrees.len(), 3);
        assert_eq!(worktrees[0].path, "/repo");
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(worktrees[1].branch.as_deref(), Some("forge/agent/tester/run1"));
        assert_eq!(worktrees[2].head_sha, "789abc");
        assert_eq!(worktrees[2].branch, None);
    }

    /// A minimal real git repository in a tempdir, with one commit on its
    /// default branch — shared setup for the worktree lifecycle tests below,
    /// which shell out to the real `git` binary rather than mocking it.
    fn init_test_repo() -> (tempfile::TempDir, GitCliService, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = GitCliService {
            git_path: PathBuf::from("git"),
        };
        service.run(dir.path(), &["init"]).expect("git init");
        service
            .run(dir.path(), &["config", "user.email", "test@example.com"])
            .expect("git config email");
        service
            .run(dir.path(), &["config", "user.name", "Test"])
            .expect("git config name");
        std::fs::write(dir.path().join("README.md"), "hello\n").expect("write README");
        service.run(dir.path(), &["add", "."]).expect("git add");
        service
            .run(dir.path(), &["commit", "-m", "init"])
            .expect("git commit");
        let branch = service
            .current_branch(dir.path())
            .expect("current_branch")
            .expect("not detached");
        (dir, service, branch)
    }

    #[test]
    fn add_worktree_creates_branch_and_checkout() {
        let (repo_dir, service, base_branch) = init_test_repo();
        let worktree_path = repo_dir.path().join(".forge-workspace/worktrees/run1");

        service
            .add_worktree(repo_dir.path(), &worktree_path, "forge/agent/test/run1", &base_branch)
            .expect("add_worktree should succeed");

        assert!(worktree_path.join("README.md").exists());

        let worktrees = service.list_worktrees(repo_dir.path()).expect("list_worktrees");
        assert_eq!(worktrees.len(), 2, "primary + one agent worktree");
        assert!(worktrees
            .iter()
            .any(|w| w.branch.as_deref() == Some("forge/agent/test/run1")));
    }

    #[test]
    fn add_worktree_fails_with_real_git_error_on_duplicate_branch() {
        let (repo_dir, service, base_branch) = init_test_repo();
        let worktree_path = repo_dir.path().join(".forge-workspace/worktrees/run1");
        service
            .add_worktree(repo_dir.path(), &worktree_path, "forge/agent/test/run1", &base_branch)
            .expect("first add_worktree should succeed");

        let second_path = repo_dir.path().join(".forge-workspace/worktrees/run2");
        let err = service
            .add_worktree(repo_dir.path(), &second_path, "forge/agent/test/run1", &base_branch)
            .expect_err("duplicate branch name should fail");
        // Real git stderr, not a generic message — should mention the branch.
        assert!(err.to_string().contains("forge/agent/test/run1"));
    }

    #[test]
    fn remove_worktree_removes_clean_worktree() {
        let (repo_dir, service, base_branch) = init_test_repo();
        let worktree_path = repo_dir.path().join(".forge-workspace/worktrees/run1");
        service
            .add_worktree(repo_dir.path(), &worktree_path, "forge/agent/test/run1", &base_branch)
            .expect("add_worktree");

        service
            .remove_worktree(repo_dir.path(), &worktree_path)
            .expect("remove_worktree should succeed on a clean worktree");

        let worktrees = service.list_worktrees(repo_dir.path()).expect("list_worktrees");
        assert_eq!(worktrees.len(), 1, "only the primary worktree remains");
    }

    #[test]
    fn remove_worktree_refuses_dirty_worktree_without_force() {
        let (repo_dir, service, base_branch) = init_test_repo();
        let worktree_path = repo_dir.path().join(".forge-workspace/worktrees/run1");
        service
            .add_worktree(repo_dir.path(), &worktree_path, "forge/agent/test/run1", &base_branch)
            .expect("add_worktree");

        // Uncommitted change inside the worktree.
        std::fs::write(worktree_path.join("README.md"), "changed\n").expect("write");

        let result = service.remove_worktree(repo_dir.path(), &worktree_path);
        assert!(result.is_err(), "dirty worktree removal must fail, not be force-removed");

        // The worktree should still be there since removal was refused.
        let worktrees = service.list_worktrees(repo_dir.path()).expect("list_worktrees");
        assert_eq!(worktrees.len(), 2, "dirty worktree was not removed");
    }
}

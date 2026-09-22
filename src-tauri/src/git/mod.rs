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

/// One conflicted path after a failed merge attempt, with git's own raw
/// two-character `XY` "unmerged" status code from `git status --porcelain=v2`
/// (`UU` both modified, `AA` both added, `UD`/`DU` deleted on one side, ...) —
/// shown verbatim rather than translated, since the exact combination matters
/// for a human (or the M15 conflict resolver) deciding how to resolve it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictedFile {
    pub path: String,
    pub status_code: String,
}

/// The result of a real merge attempt (`GitService::merge_branch`) or a
/// conflict-detection dry run (`GitService::merge_conflict_dry_run`) — M15.
/// Deliberately not a `bool` + separate file list: `Conflicts` always carries
/// the files it found, so a caller can never observe "there was a conflict"
/// without also knowing which files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Clean,
    Conflicts(Vec<ConflictedFile>),
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

    /// M15: every currently-unmerged path in `repo_root`'s working tree
    /// (i.e. mid-conflicted-merge), parsed from the exact same
    /// `git status --porcelain=v2` output `status` already parses — an
    /// unmerged path always shows up as one of `parse_status`'s `unstaged`
    /// entries with a **two**-character status code (`UU`, `AA`, `UD`, ...),
    /// unlike every ordinary staged/unstaged entry (always one character) —
    /// so this is a thin filter over `status`, not a second parser. Empty
    /// when there's no merge in progress, or a merge in progress has nothing
    /// left unresolved.
    fn conflicted_files(&self, repo_root: &Path) -> AppResult<Vec<ConflictedFile>>;

    /// Performs a **real** merge of `branch` into `into` in the primary
    /// checkout at `repo_root` (never a worktree — worktrees are disposable
    /// agent workspaces, per the Phase 4 plan; merges always land in the
    /// primary checkout): checks out `into` first (surfacing a real git
    /// error honestly if that fails, e.g. uncommitted local changes — this
    /// never force-discards anything), then runs
    /// `git merge --no-ff --no-edit <branch>`. A clean merge is committed
    /// immediately (`--no-ff` always creates a real merge commit; `--no-edit`
    /// avoids blocking on an interactive editor) — that's the whole point of
    /// calling this rather than the dry run. On conflicts, the merge is left
    /// **in progress** (conflict markers in the working tree, `MERGE_HEAD`
    /// set) for the caller to resolve (`abort_merge`, or M15's AI conflict
    /// resolver) — never auto-resolved here.
    fn merge_branch(&self, repo_root: &Path, branch: &str, into: &str) -> AppResult<MergeOutcome>;

    /// M15's core "detect before doing" mechanism: checks out `into`, then
    /// runs `git merge --no-commit --no-ff <branch>` and **unconditionally**
    /// runs `git merge --abort` afterward — regardless of whether the merge
    /// came back clean or conflicted — before returning. `--no-commit` means
    /// even a clean merge only stages its result rather than creating a real
    /// commit, so the trailing `merge --abort` always has something to
    /// cleanly undo and the repository is left exactly as it was found
    /// either way. This is how `get_merge_readiness` answers "would this
    /// conflict?" without ever leaving the primary checkout mid-merge or
    /// with staged changes, and without ever calling the real,
    /// commit-producing `merge_branch` speculatively.
    fn merge_conflict_dry_run(&self, repo_root: &Path, branch: &str, into: &str) -> AppResult<MergeOutcome>;

    /// `git merge --abort` — the honest "back out" path exposed to the user
    /// when a real (non-dry-run) `merge_branch` left the primary checkout
    /// conflicted and they don't want to resolve it (manually, or via the AI
    /// conflict resolver).
    fn abort_merge(&self, repo_root: &Path) -> AppResult<()>;

    /// Stages exactly `paths` (`git add -- <paths...>`) — used once the AI
    /// conflict resolver (or a human) has resolved every conflict marker in
    /// those specific files, deliberately never a blanket `git add -A` that
    /// could stage unrelated working-tree state.
    fn stage_paths(&self, repo_root: &Path, paths: &[String]) -> AppResult<()>;

    /// `git commit -m <message>` — finishes an in-progress merge once its
    /// conflicts have been resolved and staged. Every call site of this in
    /// M15 (`agent::conflict_resolver`) verifies `conflicted_files` is
    /// genuinely empty first — this method itself does no such check, it
    /// just commits whatever's staged.
    fn commit(&self, repo_root: &Path, message: &str) -> AppResult<()>;
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

    /// Like `run`, but hands back the full `Output` (stdout/stderr/exit
    /// status) regardless of exit code, rather than erroring on a non-zero
    /// exit — used for `merge`/`merge --no-commit` attempts (M15), where a
    /// non-zero exit is an *expected*, meaningful outcome (a real conflict)
    /// the caller needs to distinguish from an actual git error, not
    /// something to collapse into a generic `Err` the way `run` does.
    fn run_output(&self, repo_path: &Path, args: &[&str]) -> AppResult<std::process::Output> {
        Command::new(&self.git_path)
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
            })
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

    fn conflicted_files(&self, repo_root: &Path) -> AppResult<Vec<ConflictedFile>> {
        let status = self.status(repo_root)?;
        Ok(status
            .unstaged
            .into_iter()
            .filter(|e| e.status_code.chars().count() == 2)
            .map(|e| ConflictedFile { path: e.path, status_code: e.status_code })
            .collect())
    }

    fn merge_branch(&self, repo_root: &Path, branch: &str, into: &str) -> AppResult<MergeOutcome> {
        // Land the merge on `into` regardless of whatever happens to be
        // checked out already — a real git error here (e.g. uncommitted
        // local changes blocking the checkout) surfaces honestly rather than
        // silently merging onto the wrong branch.
        self.run(repo_root, &["checkout", into])?;

        let output = self.run_output(repo_root, &["merge", "--no-ff", "--no-edit", branch])?;
        if output.status.success() {
            return Ok(MergeOutcome::Clean);
        }

        let conflicts = self.conflicted_files(repo_root)?;
        if conflicts.is_empty() {
            // The merge failed for some other real reason (unknown branch,
            // an unrelated-histories error, ...) — not a conflict, so
            // there's nothing left mid-merge to report as one. Surface
            // git's own stderr rather than a generic message.
            return Err(AppError::Other(merge_failure_message(&output)));
        }
        Ok(MergeOutcome::Conflicts(conflicts))
    }

    fn merge_conflict_dry_run(&self, repo_root: &Path, branch: &str, into: &str) -> AppResult<MergeOutcome> {
        self.run(repo_root, &["checkout", into])?;
        let output = self.run_output(repo_root, &["merge", "--no-commit", "--no-ff", branch])?;

        let result = if output.status.success() {
            Ok(MergeOutcome::Clean)
        } else {
            let conflicts = self.conflicted_files(repo_root)?;
            if conflicts.is_empty() {
                Err(AppError::Other(merge_failure_message(&output)))
            } else {
                Ok(MergeOutcome::Conflicts(conflicts))
            }
        };

        // Unconditional, regardless of `result` above — the whole point of
        // the dry run is that the primary checkout is never left mid-merge
        // or holding staged changes, whether this came back clean,
        // conflicted, or erroring for some other reason. Best-effort: if
        // there was nothing to abort (e.g. the checkout itself never got far
        // enough to start a merge), this is simply a no-op error that's
        // deliberately swallowed rather than shadowing the real `result`.
        let _ = self.run(repo_root, &["merge", "--abort"]);
        result
    }

    fn abort_merge(&self, repo_root: &Path) -> AppResult<()> {
        self.run(repo_root, &["merge", "--abort"]).map(|_| ())
    }

    fn stage_paths(&self, repo_root: &Path, paths: &[String]) -> AppResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run(repo_root, &args)?;
        Ok(())
    }

    fn commit(&self, repo_root: &Path, message: &str) -> AppResult<()> {
        self.run(repo_root, &["commit", "-m", message])?;
        Ok(())
    }
}

/// Formats a failed (non-conflict) `git merge`/`git merge --no-commit`
/// attempt's real stderr into an error message — shared by `merge_branch`
/// and `merge_conflict_dry_run` so both report a git failure the same way.
fn merge_failure_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("git merge failed (exit {})", output.status)
    } else {
        stderr
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

    // -- M15: merge_branch / merge_conflict_dry_run / abort_merge -----------
    // Real git, in tempdir repos — same pattern as the worktree tests above.

    /// Creates a real branch off the current `HEAD` and commits `content` to
    /// `file_name` on it, leaving the repo checked back out on `base_branch`
    /// afterward (mirroring how a real agent worktree's commits would look
    /// from the primary checkout's point of view before a merge attempt).
    fn commit_on_new_branch(service: &GitCliService, repo_dir: &Path, base_branch: &str, branch: &str, file_name: &str, content: &str) {
        service.run(repo_dir, &["checkout", "-b", branch]).expect("checkout -b");
        std::fs::write(repo_dir.join(file_name), content).expect("write file");
        service.run(repo_dir, &["add", "."]).expect("add");
        service.run(repo_dir, &["commit", "-m", &format!("commit on {branch}")]).expect("commit");
        service.run(repo_dir, &["checkout", base_branch]).expect("checkout back to base");
    }

    #[test]
    fn merge_branch_merges_cleanly_when_changes_do_not_conflict() {
        let (repo_dir, service, base_branch) = init_test_repo();
        commit_on_new_branch(&service, repo_dir.path(), &base_branch, "feature", "new-file.txt", "hello from feature\n");

        let outcome = service.merge_branch(repo_dir.path(), "feature", &base_branch).expect("merge_branch should succeed");
        assert_eq!(outcome, MergeOutcome::Clean);

        // The merge actually landed: the feature branch's file is now on the
        // base branch, and a real merge commit exists.
        assert!(repo_dir.path().join("new-file.txt").exists());
        let log = service.log(repo_dir.path(), 5).expect("log");
        assert!(log.iter().any(|c| c.subject.to_lowercase().contains("merge")));
        assert!(service.conflicted_files(repo_dir.path()).expect("conflicted_files").is_empty());
    }

    #[test]
    fn merge_branch_detects_a_real_conflict_and_leaves_it_for_the_caller() {
        let (repo_dir, service, base_branch) = init_test_repo();
        // Both branches edit the same line of the same file — a genuine
        // conflict, not a simulated one.
        std::fs::write(repo_dir.path().join("shared.txt"), "base content\n").expect("write");
        service.run(repo_dir.path(), &["add", "."]).expect("add");
        service.run(repo_dir.path(), &["commit", "-m", "add shared.txt"]).expect("commit");

        commit_on_new_branch(&service, repo_dir.path(), &base_branch, "feature", "shared.txt", "feature branch's version\n");
        std::fs::write(repo_dir.path().join("shared.txt"), "base branch's version\n").expect("write");
        service.run(repo_dir.path(), &["add", "."]).expect("add");
        service.run(repo_dir.path(), &["commit", "-m", "diverge on base"]).expect("commit");

        let outcome = service.merge_branch(repo_dir.path(), "feature", &base_branch).expect("merge_branch should return Conflicts, not Err");
        match outcome {
            MergeOutcome::Conflicts(files) => {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].path, "shared.txt");
                assert_eq!(files[0].status_code.chars().count(), 2);
            }
            MergeOutcome::Clean => panic!("expected a real conflict"),
        }

        // The merge is genuinely left in progress — conflict markers are on
        // disk, and `conflicted_files` (the same status-based detection)
        // agrees.
        let on_disk = std::fs::read_to_string(repo_dir.path().join("shared.txt")).expect("read shared.txt");
        assert!(on_disk.contains("<<<<<<<"));
        assert_eq!(service.conflicted_files(repo_dir.path()).expect("conflicted_files").len(), 1);

        // abort_merge cleanly backs out, leaving no conflict behind.
        service.abort_merge(repo_dir.path()).expect("abort_merge");
        assert!(service.conflicted_files(repo_dir.path()).expect("conflicted_files").is_empty());
        let on_disk_after_abort = std::fs::read_to_string(repo_dir.path().join("shared.txt")).expect("read shared.txt");
        assert!(!on_disk_after_abort.contains("<<<<<<<"));
    }

    #[test]
    fn merge_conflict_dry_run_detects_conflicts_without_leaving_the_repo_mid_merge() {
        let (repo_dir, service, base_branch) = init_test_repo();
        std::fs::write(repo_dir.path().join("shared.txt"), "base content\n").expect("write");
        service.run(repo_dir.path(), &["add", "."]).expect("add");
        service.run(repo_dir.path(), &["commit", "-m", "add shared.txt"]).expect("commit");

        commit_on_new_branch(&service, repo_dir.path(), &base_branch, "feature", "shared.txt", "feature branch's version\n");
        std::fs::write(repo_dir.path().join("shared.txt"), "base branch's version\n").expect("write");
        service.run(repo_dir.path(), &["add", "."]).expect("add");
        service.run(repo_dir.path(), &["commit", "-m", "diverge on base"]).expect("commit");

        let outcome =
            service.merge_conflict_dry_run(repo_dir.path(), "feature", &base_branch).expect("merge_conflict_dry_run should not error");
        match outcome {
            MergeOutcome::Conflicts(files) => assert_eq!(files[0].path, "shared.txt"),
            MergeOutcome::Clean => panic!("expected a real conflict"),
        }

        // The dry run's whole contract: nothing is left behind — no
        // mid-merge state, no conflict markers, no staged changes.
        assert!(service.conflicted_files(repo_dir.path()).expect("conflicted_files").is_empty());
        let status = service.status(repo_dir.path()).expect("status");
        assert!(status.staged.is_empty() && status.unstaged.is_empty());
        let on_disk = std::fs::read_to_string(repo_dir.path().join("shared.txt")).expect("read shared.txt");
        assert!(!on_disk.contains("<<<<<<<"), "dry run must never leave conflict markers on disk");
    }

    #[test]
    fn merge_conflict_dry_run_reports_clean_and_still_leaves_nothing_committed() {
        let (repo_dir, service, base_branch) = init_test_repo();
        commit_on_new_branch(&service, repo_dir.path(), &base_branch, "feature", "new-file.txt", "hello\n");

        let outcome = service.merge_conflict_dry_run(repo_dir.path(), "feature", &base_branch).expect("merge_conflict_dry_run");
        assert_eq!(outcome, MergeOutcome::Clean);

        // Nothing was actually merged — a dry run must never commit.
        assert!(!repo_dir.path().join("new-file.txt").exists());
        let log = service.log(repo_dir.path(), 5).expect("log");
        assert!(!log.iter().any(|c| c.subject.to_lowercase().contains("merge")));
    }

    #[test]
    fn stage_paths_then_commit_finishes_a_resolved_merge() {
        let (repo_dir, service, base_branch) = init_test_repo();
        std::fs::write(repo_dir.path().join("shared.txt"), "base content\n").expect("write");
        service.run(repo_dir.path(), &["add", "."]).expect("add");
        service.run(repo_dir.path(), &["commit", "-m", "add shared.txt"]).expect("commit");

        commit_on_new_branch(&service, repo_dir.path(), &base_branch, "feature", "shared.txt", "feature branch's version\n");
        std::fs::write(repo_dir.path().join("shared.txt"), "base branch's version\n").expect("write");
        service.run(repo_dir.path(), &["add", "."]).expect("add");
        service.run(repo_dir.path(), &["commit", "-m", "diverge on base"]).expect("commit");

        let outcome = service.merge_branch(repo_dir.path(), "feature", &base_branch).expect("merge_branch");
        assert!(matches!(outcome, MergeOutcome::Conflicts(_)));

        // Resolve it exactly the way the conflict resolver would: overwrite
        // the file with real, marker-free resolved content.
        std::fs::write(repo_dir.path().join("shared.txt"), "resolved content\n").expect("write resolved content");
        assert!(service.conflicted_files(repo_dir.path()).expect("conflicted_files — still unmerged until staged").len() == 1);

        service.stage_paths(repo_dir.path(), &["shared.txt".to_string()]).expect("stage_paths");
        service.commit(repo_dir.path(), "Merge feature into base — resolved conflicts").expect("commit");

        assert!(service.conflicted_files(repo_dir.path()).expect("conflicted_files").is_empty());
        let on_disk = std::fs::read_to_string(repo_dir.path().join("shared.txt")).expect("read shared.txt");
        assert_eq!(on_disk, "resolved content\n");
        let log = service.log(repo_dir.path(), 5).expect("log");
        assert!(log.iter().any(|c| c.subject.contains("resolved conflicts")));
    }
}

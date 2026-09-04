//! Git operations: repository discovery, worktree management for agent
//! isolation, and diff generation for the Changes tab. Shells out to the
//! system `git` binary (resolved via `os_adapter::OperatingSystemAdapter::resolve_executable`)
//! rather than linking libgit2, to stay compatible with the user's own git
//! config/credentials/hooks.

use std::path::{Path, PathBuf};

use crate::error::AppResult;

#[derive(Debug, Clone)]
pub struct GitStatusEntry {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct GitDiff {
    pub path: String,
    pub original: String,
    pub modified: String,
}

/// True if `path` (or an ancestor) contains a `.git` directory/file.
pub fn is_git_repository(_path: &Path) -> bool {
    todo!("M2: walk ancestors looking for .git")
}

/// Repository root for `path`, via `git rev-parse --show-toplevel`.
pub fn discover_root(_path: &Path) -> AppResult<PathBuf> {
    todo!("M2: git rev-parse --show-toplevel")
}

/// Current branch name, via `git rev-parse --abbrev-ref HEAD`.
pub fn current_branch(_repo_root: &Path) -> AppResult<String> {
    todo!("M2: git rev-parse --abbrev-ref HEAD")
}

/// Working-tree status, via `git status --porcelain=v1`.
pub fn status(_repo_root: &Path) -> AppResult<Vec<GitStatusEntry>> {
    todo!("M2: git status --porcelain=v1")
}

/// Creates a new worktree at `worktree_path` on a new branch `branch_name`,
/// for isolating one agent's changes from the primary checkout.
pub fn add_worktree(
    _repo_root: &Path,
    _worktree_path: &Path,
    _branch_name: &str,
    _base_branch: &str,
) -> AppResult<()> {
    todo!("M3: git worktree add -b <branch_name> <worktree_path> <base_branch>")
}

/// Removes a worktree previously created by `add_worktree`.
pub fn remove_worktree(_repo_root: &Path, _worktree_path: &Path) -> AppResult<()> {
    todo!("M3: git worktree remove <worktree_path>")
}

/// Diff for a single file between two refs (or working tree vs. HEAD).
pub fn diff_file(_repo_root: &Path, _path: &str, _base_ref: &str) -> AppResult<GitDiff> {
    todo!("M2: git show <base_ref>:<path> + read working tree file")
}

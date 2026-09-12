//! Commands backing `src/stores/useRepositoryStore.ts`, the TopBar's branch
//! indicator, and (eventually) the Changes tab.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::commands::run_blocking;
use crate::db::models::{Repository, Workspace};
use crate::error::AppResult;
use crate::git::{BranchInfo, CommitInfo, GitFileDiff, GitStatus};
use crate::state::AppState;

/// Working-tree status (branch + staged/unstaged/untracked files) for the
/// repository rooted at `repo_path`.
#[tauri::command]
pub async fn git_status(app: AppHandle, repo_path: String) -> Result<GitStatus, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.git_service.status(&PathBuf::from(repo_path))
    })
    .await
}

/// All local branches for the repository rooted at `repo_path`.
#[tauri::command]
pub async fn git_branches(app: AppHandle, repo_path: String) -> Result<Vec<BranchInfo>, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.git_service.branches(&PathBuf::from(repo_path))
    })
    .await
}

/// The currently checked-out branch for the repository rooted at
/// `repo_path`, or `null` if HEAD is detached.
#[tauri::command]
pub async fn git_current_branch(app: AppHandle, repo_path: String) -> Result<Option<String>, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.git_service.current_branch(&PathBuf::from(repo_path))
    })
    .await
}

/// Full before/after text for `file_path` (relative to `repo_path`), for the
/// Changes tab's Monaco diff viewer. Handles new and deleted files rather
/// than erroring on either — see `GitFileDiff`'s field docs.
#[tauri::command]
pub async fn git_diff_file(app: AppHandle, repo_path: String, file_path: String) -> Result<GitFileDiff, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.git_service.diff_file(&PathBuf::from(repo_path), &file_path)
    })
    .await
}

/// The `limit` most recent commits for the repository rooted at `repo_path`.
#[tauri::command]
pub async fn git_log(app: AppHandle, repo_path: String, limit: u32) -> Result<Vec<CommitInfo>, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.git_service.log(&PathBuf::from(repo_path), limit)
    })
    .await
}

#[tauri::command]
pub fn open_repository(_state: State<AppState>, _root_path: String) -> AppResult<Repository> {
    todo!("M2+: superseded by project_commands::open_project for the Projects flow; kept for a future direct-repo-only flow")
}

#[tauri::command]
pub fn list_workspaces(_state: State<AppState>, _repository_id: String) -> AppResult<Vec<Workspace>> {
    todo!("M3: db::repository::workspaces::list_for_repository")
}

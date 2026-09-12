//! Commands backing `src/stores/useRepositoryStore.ts`, the TopBar's branch
//! indicator, and (eventually) the Changes tab.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::commands::run_blocking;
use crate::db::models::{Repository, Workspace};
use crate::error::AppResult;
use crate::git::{BranchInfo, GitStatus};
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

#[tauri::command]
pub fn open_repository(_state: State<AppState>, _root_path: String) -> AppResult<Repository> {
    todo!("M2+: superseded by project_commands::open_project for the Projects flow; kept for a future direct-repo-only flow")
}

#[tauri::command]
pub fn list_workspaces(_state: State<AppState>, _repository_id: String) -> AppResult<Vec<Workspace>> {
    todo!("M3: db::repository::workspaces::list_for_repository")
}

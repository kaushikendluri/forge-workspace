//! Commands backing `src/stores/useRepositoryStore.ts` and the Changes tab.

use tauri::State;

use crate::db::models::{Repository, Workspace};
use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn open_repository(_state: State<AppState>, _root_path: String) -> AppResult<Repository> {
    todo!("M2: git::discover_root + db::repository::repositories::create")
}

#[tauri::command]
pub fn list_workspaces(_state: State<AppState>, _repository_id: String) -> AppResult<Vec<Workspace>> {
    todo!("M3: db::repository::workspaces::list_for_repository")
}

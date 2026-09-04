//! Commands backing `src/stores/useProjectStore.ts` and the
//! Dashboard/Projects routes. See `src/lib/tauri.ts`'s `CommandMap` for the
//! matching frontend-side request/response types.

use tauri::State;

use crate::db::models::{Agent, Project, Task};
use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn list_projects(_state: State<AppState>) -> AppResult<Vec<Project>> {
    todo!("M2: db::repository::projects::list")
}

#[tauri::command]
pub fn get_project(_state: State<AppState>, _project_id: String) -> AppResult<Option<Project>> {
    todo!("M2: db::repository::projects::get")
}

#[tauri::command]
pub fn create_project(
    _state: State<AppState>,
    _name: String,
    _description: Option<String>,
) -> AppResult<Project> {
    todo!("M2: db::repository::projects::create")
}

#[tauri::command]
pub fn list_agents(_state: State<AppState>, _project_id: String) -> AppResult<Vec<Agent>> {
    todo!("M5: agents land with the agent-run execution milestone")
}

#[tauri::command]
pub fn list_tasks(_state: State<AppState>, _project_id: String) -> AppResult<Vec<Task>> {
    todo!("M2: db::repository::tasks::list_for_project")
}

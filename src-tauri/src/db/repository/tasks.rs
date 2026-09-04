//! CRUD for the `tasks` table.

use rusqlite::Connection;

use crate::db::models::{Task, TaskStatus};
use crate::error::AppResult;

pub fn list_for_project(_conn: &Connection, _project_id: &str) -> AppResult<Vec<Task>> {
    todo!("M2: SELECT * FROM tasks WHERE project_id = ?1 ORDER BY created_at")
}

pub fn create(_conn: &Connection, _project_id: &str, _title: &str, _description: Option<&str>) -> AppResult<Task> {
    todo!("M2: INSERT INTO tasks (...) VALUES (...)")
}

pub fn update_status(_conn: &Connection, _id: &str, _status: TaskStatus) -> AppResult<()> {
    todo!("M2: UPDATE tasks SET status = ?2, updated_at = now() WHERE id = ?1")
}

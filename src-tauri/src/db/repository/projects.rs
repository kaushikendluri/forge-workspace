//! CRUD for the `projects` table. Backs `commands::project_commands`.

use rusqlite::Connection;

use crate::db::models::Project;
use crate::error::AppResult;

pub fn list(_conn: &Connection) -> AppResult<Vec<Project>> {
    todo!("M2: SELECT * FROM projects ORDER BY updated_at DESC")
}

pub fn get(_conn: &Connection, _id: &str) -> AppResult<Option<Project>> {
    todo!("M2: SELECT * FROM projects WHERE id = ?1")
}

pub fn create(_conn: &Connection, _name: &str, _description: Option<&str>) -> AppResult<Project> {
    todo!("M2: INSERT INTO projects (...) VALUES (...)")
}

pub fn touch_last_opened(_conn: &Connection, _id: &str) -> AppResult<()> {
    todo!("M2: UPDATE projects SET last_opened_at = now() WHERE id = ?1")
}

pub fn delete(_conn: &Connection, _id: &str) -> AppResult<()> {
    todo!("M2: DELETE FROM projects WHERE id = ?1 (cascades to repositories/agents/tasks)")
}

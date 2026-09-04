//! CRUD for the `repositories` table (git repository roots registered
//! against a project). Backs `commands::git_commands`.

use rusqlite::Connection;

use crate::db::models::Repository;
use crate::error::AppResult;

pub fn list_for_project(_conn: &Connection, _project_id: &str) -> AppResult<Vec<Repository>> {
    todo!("M2: SELECT * FROM repositories WHERE project_id = ?1")
}

pub fn get(_conn: &Connection, _id: &str) -> AppResult<Option<Repository>> {
    todo!("M2: SELECT * FROM repositories WHERE id = ?1")
}

pub fn get_by_root_path(_conn: &Connection, _root_path: &str) -> AppResult<Option<Repository>> {
    todo!("M2: SELECT * FROM repositories WHERE root_path = ?1")
}

pub fn create(
    _conn: &Connection,
    _project_id: &str,
    _root_path: &str,
    _remote_url: Option<&str>,
    _default_branch: &str,
) -> AppResult<Repository> {
    todo!("M2: INSERT INTO repositories (...) VALUES (...)")
}

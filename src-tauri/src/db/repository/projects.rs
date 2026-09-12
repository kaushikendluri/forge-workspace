//! CRUD for the `projects` table. Backs `commands::project_commands`.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::Project;
use crate::error::AppResult;

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        last_opened_at: row.get(5)?,
    })
}

const SELECT_COLUMNS: &str = "id, name, description, created_at, updated_at, last_opened_at";

pub fn list_all(conn: &Connection) -> AppResult<Vec<Project>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM projects ORDER BY updated_at DESC"
    ))?;
    let rows = stmt
        .query_map([], row_to_project)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<Project>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM projects WHERE id = ?1"),
        params![id],
        row_to_project,
    )
    .optional()
    .map_err(Into::into)
}

/// Inserts a new project row and returns it.
pub fn insert(conn: &Connection, name: &str, description: Option<&str>) -> AppResult<Project> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO projects (id, name, description, created_at, updated_at, last_opened_at)
         VALUES (?1, ?2, ?3, ?4, ?4, NULL)",
        params![id, name, description, now],
    )?;
    Ok(Project {
        id,
        name: name.to_string(),
        description: description.map(|s| s.to_string()),
        created_at: now.clone(),
        updated_at: now,
        last_opened_at: None,
    })
}

/// Stamps `last_opened_at` (and `updated_at`) with the current time.
pub fn update_last_opened(conn: &Connection, id: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE projects SET last_opened_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
    Ok(())
}

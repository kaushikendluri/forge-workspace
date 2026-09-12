//! CRUD for the `repositories` table (git repository roots registered
//! against a project). Backs `commands::project_commands` and
//! `commands::git_commands`.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::Repository;
use crate::error::AppResult;

fn row_to_repository(row: &rusqlite::Row<'_>) -> rusqlite::Result<Repository> {
    Ok(Repository {
        id: row.get(0)?,
        project_id: row.get(1)?,
        root_path: row.get(2)?,
        remote_url: row.get(3)?,
        default_branch: row.get(4)?,
        vcs_type: row.get(5)?,
        created_at: row.get(6)?,
    })
}

const SELECT_COLUMNS: &str = "id, project_id, root_path, remote_url, default_branch, vcs_type, created_at";

pub fn list_all(conn: &Connection) -> AppResult<Vec<Repository>> {
    let mut stmt = conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM repositories"))?;
    let rows = stmt
        .query_map([], row_to_repository)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn list_for_project(conn: &Connection, project_id: &str) -> AppResult<Vec<Repository>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM repositories WHERE project_id = ?1"
    ))?;
    let rows = stmt
        .query_map(params![project_id], row_to_repository)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A project currently has exactly one repository (Phase 1); this is the
/// convenience accessor `project_commands` joins against when building the
/// `ProjectDto` the frontend renders.
pub fn get_by_project_id(conn: &Connection, project_id: &str) -> AppResult<Option<Repository>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM repositories WHERE project_id = ?1 LIMIT 1"),
        params![project_id],
        row_to_repository,
    )
    .optional()
    .map_err(Into::into)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<Repository>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM repositories WHERE id = ?1"),
        params![id],
        row_to_repository,
    )
    .optional()
    .map_err(Into::into)
}

pub fn get_by_root_path(conn: &Connection, root_path: &str) -> AppResult<Option<Repository>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM repositories WHERE root_path = ?1"),
        params![root_path],
        row_to_repository,
    )
    .optional()
    .map_err(Into::into)
}

pub fn insert(
    conn: &Connection,
    project_id: &str,
    root_path: &str,
    remote_url: Option<&str>,
    default_branch: &str,
) -> AppResult<Repository> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO repositories (id, project_id, root_path, remote_url, default_branch, vcs_type, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'git', ?6)",
        params![id, project_id, root_path, remote_url, default_branch, now],
    )?;
    Ok(Repository {
        id,
        project_id: project_id.to_string(),
        root_path: root_path.to_string(),
        remote_url: remote_url.map(|s| s.to_string()),
        default_branch: default_branch.to_string(),
        vcs_type: "git".to_string(),
        created_at: now,
    })
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM repositories WHERE id = ?1", params![id])?;
    Ok(())
}

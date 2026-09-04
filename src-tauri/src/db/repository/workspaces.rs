//! CRUD for the `workspaces` table (git-worktree-backed checkouts, one
//! `primary` per repository plus one `agent` worktree per running agent).

use rusqlite::Connection;

use crate::db::models::Workspace;
use crate::error::AppResult;

pub fn list_for_repository(_conn: &Connection, _repository_id: &str) -> AppResult<Vec<Workspace>> {
    todo!("M2/M3: SELECT * FROM workspaces WHERE repository_id = ?1 AND status = 'active'")
}

pub fn get(_conn: &Connection, _id: &str) -> AppResult<Option<Workspace>> {
    todo!("M2/M3: SELECT * FROM workspaces WHERE id = ?1")
}

pub fn create(
    _conn: &Connection,
    _repository_id: &str,
    _kind: &str,
    _path: &str,
    _branch_name: &str,
    _base_branch: Option<&str>,
) -> AppResult<Workspace> {
    todo!("M3: INSERT INTO workspaces (...) VALUES (...) after `git worktree add`")
}

pub fn mark_removed(_conn: &Connection, _id: &str) -> AppResult<()> {
    todo!("M3: UPDATE workspaces SET status = 'removed', removed_at = now() WHERE id = ?1")
}

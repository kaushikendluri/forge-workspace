//! CRUD for the `workspaces` table (git-worktree-backed checkouts, one
//! `primary` per repository plus one `agent` worktree per running agent).

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{Workspace, WorkspaceKind, WorkspaceStatus};
use crate::error::AppResult;

fn kind_str(kind: WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Primary => "primary",
        WorkspaceKind::Agent => "agent",
    }
}

fn parse_kind(s: &str) -> WorkspaceKind {
    match s {
        "primary" => WorkspaceKind::Primary,
        _ => WorkspaceKind::Agent,
    }
}

fn parse_status(s: &str) -> WorkspaceStatus {
    match s {
        "removed" => WorkspaceStatus::Removed,
        _ => WorkspaceStatus::Active,
    }
}

fn row_to_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workspace> {
    let kind: String = row.get(3)?;
    let status: String = row.get(8)?;
    Ok(Workspace {
        id: row.get(0)?,
        repository_id: row.get(1)?,
        agent_run_id: row.get(2)?,
        kind: parse_kind(&kind),
        path: row.get(4)?,
        branch_name: row.get(5)?,
        base_branch: row.get(6)?,
        base_commit_sha: row.get(7)?,
        status: parse_status(&status),
        created_at: row.get(9)?,
        removed_at: row.get(10)?,
    })
}

const SELECT_COLUMNS: &str = "id, repository_id, agent_run_id, kind, path, branch_name, \
    base_branch, base_commit_sha, status, created_at, removed_at";

/// Active workspaces for `repository_id`, most recently created first.
pub fn list_for_repository(conn: &Connection, repository_id: &str) -> AppResult<Vec<Workspace>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM workspaces WHERE repository_id = ?1 AND status = 'active' ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![repository_id], row_to_workspace)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<Workspace>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM workspaces WHERE id = ?1"),
        params![id],
        row_to_workspace,
    )
    .optional()
    .map_err(Into::into)
}

/// Inserts a new `active` workspace row and returns it. Callers are
/// responsible for having already created the worktree on disk (via
/// `GitService::add_worktree`) before this is called — this only records it.
#[allow(clippy::too_many_arguments)]
pub fn insert(
    conn: &Connection,
    repository_id: &str,
    agent_run_id: Option<&str>,
    kind: WorkspaceKind,
    path: &str,
    branch_name: &str,
    base_branch: Option<&str>,
    base_commit_sha: Option<&str>,
) -> AppResult<Workspace> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO workspaces (id, repository_id, agent_run_id, kind, path, branch_name, base_branch, base_commit_sha, status, created_at, removed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, NULL)",
        params![
            id,
            repository_id,
            agent_run_id,
            kind_str(kind),
            path,
            branch_name,
            base_branch,
            base_commit_sha,
            now
        ],
    )?;
    Ok(Workspace {
        id,
        repository_id: repository_id.to_string(),
        agent_run_id: agent_run_id.map(str::to_string),
        kind,
        path: path.to_string(),
        branch_name: branch_name.to_string(),
        base_branch: base_branch.map(str::to_string),
        base_commit_sha: base_commit_sha.map(str::to_string),
        status: WorkspaceStatus::Active,
        created_at: now,
        removed_at: None,
    })
}

/// Marks a workspace `removed` (and stamps `removed_at`). Callers are
/// responsible for having already removed the worktree on disk (via
/// `GitService::remove_worktree`) before this is called — this only records
/// it.
pub fn mark_removed(conn: &Connection, id: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE workspaces SET status = 'removed', removed_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn.execute(
            "INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')",
            [],
        )
        .expect("insert project");
        conn.execute(
            "INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')",
            [],
        )
        .expect("insert repository");
        conn
    }

    #[test]
    fn insert_then_get_by_id_round_trips() {
        let conn = setup_conn();
        let workspace = insert(
            &conn,
            "r1",
            None,
            WorkspaceKind::Agent,
            "/tmp/r1/.forge-workspace/worktrees/run1",
            "forge/agent/tester/run1",
            Some("main"),
            None,
        )
        .expect("insert");

        let fetched = get_by_id(&conn, &workspace.id)
            .expect("get_by_id")
            .expect("workspace should exist");
        assert_eq!(fetched.id, workspace.id);
        assert_eq!(fetched.branch_name, "forge/agent/tester/run1");
        assert_eq!(fetched.status, WorkspaceStatus::Active);
        assert!(fetched.removed_at.is_none());
    }

    #[test]
    fn mark_removed_excludes_from_list_for_repository() {
        let conn = setup_conn();
        let workspace = insert(
            &conn,
            "r1",
            None,
            WorkspaceKind::Agent,
            "/tmp/r1/.forge-workspace/worktrees/run1",
            "forge/agent/tester/run1",
            Some("main"),
            None,
        )
        .expect("insert");

        assert_eq!(list_for_repository(&conn, "r1").expect("list").len(), 1);

        mark_removed(&conn, &workspace.id).expect("mark_removed");

        assert_eq!(list_for_repository(&conn, "r1").expect("list").len(), 0);
        let fetched = get_by_id(&conn, &workspace.id)
            .expect("get_by_id")
            .expect("still exists, just removed");
        assert_eq!(fetched.status, WorkspaceStatus::Removed);
        assert!(fetched.removed_at.is_some());
    }
}

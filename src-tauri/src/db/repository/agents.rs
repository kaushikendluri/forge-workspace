//! CRUD for the `agents` table. Populated for real starting M5 — an agent
//! is created idle and stays idle through this milestone (a `queued`
//! `agent_runs` row records that a worktree is ready; nothing here invents a
//! `running` agent status until the execution loop lands in M6).

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{Agent, AgentStatus};
use crate::error::AppResult;

fn parse_status(s: &str) -> AgentStatus {
    match s {
        "running" => AgentStatus::Running,
        "completed" => AgentStatus::Completed,
        "failed" => AgentStatus::Failed,
        "stopped" => AgentStatus::Stopped,
        _ => AgentStatus::Idle,
    }
}

fn row_to_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<Agent> {
    let status: String = row.get(4)?;
    Ok(Agent {
        id: row.get(0)?,
        project_id: row.get(1)?,
        repository_id: row.get(2)?,
        name: row.get(3)?,
        status: parse_status(&status),
        system_prompt: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

const SELECT_COLUMNS: &str =
    "id, project_id, repository_id, name, status, system_prompt, created_at, updated_at";

/// Agents for `project_id`, most recently created first.
pub fn list_for_project(conn: &Connection, project_id: &str) -> AppResult<Vec<Agent>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM agents WHERE project_id = ?1 ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![project_id], row_to_agent)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<Agent>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM agents WHERE id = ?1"),
        params![id],
        row_to_agent,
    )
    .optional()
    .map_err(Into::into)
}

/// Inserts a new agent row (status `idle`, no system prompt yet) for
/// `repository_id` and returns it.
pub fn insert(conn: &Connection, project_id: &str, repository_id: &str, name: &str) -> AppResult<Agent> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agents (id, project_id, repository_id, name, status, system_prompt, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'idle', NULL, ?5, ?5)",
        params![id, project_id, repository_id, name, now],
    )?;
    Ok(Agent {
        id,
        project_id: project_id.to_string(),
        repository_id: repository_id.to_string(),
        name: name.to_string(),
        status: AgentStatus::Idle,
        system_prompt: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Updates an agent's `status` — called by `agent::tool_loop` alongside
/// every `agent_runs` status transition (running/completed/failed/stopped)
/// so `Agents.tsx`'s list reflects a live run without a separate poll.
pub fn set_status(conn: &Connection, id: &str, status: AgentStatus) -> AppResult<()> {
    let status_str = match status {
        AgentStatus::Idle => "idle",
        AgentStatus::Running => "running",
        AgentStatus::Completed => "completed",
        AgentStatus::Failed => "failed",
        AgentStatus::Stopped => "stopped",
    };
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agents SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, status_str, now],
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
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')", [])
            .expect("insert project");
        conn.execute(
            "INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')",
            [],
        )
        .expect("insert repository");
        conn
    }

    #[test]
    fn insert_then_list_for_project_round_trips() {
        let conn = setup_conn();
        let agent = insert(&conn, "p1", "r1", "Refactor Bot").expect("insert");
        assert_eq!(agent.status, AgentStatus::Idle);

        let listed = list_for_project(&conn, "p1").expect("list_for_project");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, agent.id);
        assert_eq!(listed[0].name, "Refactor Bot");

        let fetched = get_by_id(&conn, &agent.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.id, agent.id);
    }

    #[test]
    fn get_by_id_returns_none_for_unknown_id() {
        let conn = setup_conn();
        assert!(get_by_id(&conn, "nonexistent").expect("get_by_id").is_none());
    }

    #[test]
    fn set_status_updates_status() {
        let conn = setup_conn();
        let agent = insert(&conn, "p1", "r1", "Refactor Bot").expect("insert");
        set_status(&conn, &agent.id, AgentStatus::Running).expect("set_status");
        let fetched = get_by_id(&conn, &agent.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.status, AgentStatus::Running);
    }
}

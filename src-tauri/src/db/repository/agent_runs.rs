//! CRUD for the `agent_runs` table. `insert_queued` is called for real
//! starting M5 (one `queued` row per worktree created by
//! `commands::agent_commands::start_worktree_for_agent`) — nothing yet
//! transitions a run to `running`/`completed`/etc, since the actual model
//! call + tool loop that would do that lands in M6.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{AgentRun, AgentRunStatus, AgentRunStopReason};
use crate::error::AppResult;

fn parse_status(s: &str) -> AgentRunStatus {
    match s {
        "running" => AgentRunStatus::Running,
        "completed" => AgentRunStatus::Completed,
        "failed" => AgentRunStatus::Failed,
        "stopped" => AgentRunStatus::Stopped,
        _ => AgentRunStatus::Queued,
    }
}

fn parse_stop_reason(s: Option<String>) -> Option<AgentRunStopReason> {
    match s.as_deref() {
        Some("completed") => Some(AgentRunStopReason::Completed),
        Some("max_iterations") => Some(AgentRunStopReason::MaxIterations),
        Some("user_stopped") => Some(AgentRunStopReason::UserStopped),
        Some("error") => Some(AgentRunStopReason::Error),
        _ => None,
    }
}

fn row_to_agent_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRun> {
    let status: String = row.get(5)?;
    let stop_reason: Option<String> = row.get(6)?;
    Ok(AgentRun {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        workspace_id: row.get(2)?,
        task_prompt: row.get(3)?,
        model_id: row.get(4)?,
        status: parse_status(&status),
        stop_reason: parse_stop_reason(stop_reason),
        error_message: row.get(7)?,
        iteration_count: row.get(8)?,
        total_input_tokens: row.get(9)?,
        total_output_tokens: row.get(10)?,
        started_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

const SELECT_COLUMNS: &str = "id, agent_id, workspace_id, task_prompt, model_id, status, stop_reason, \
    error_message, iteration_count, total_input_tokens, total_output_tokens, started_at, completed_at";

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<AgentRun>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM agent_runs WHERE id = ?1"),
        params![id],
        row_to_agent_run,
    )
    .optional()
    .map_err(Into::into)
}

/// Runs for `agent_id`, most recently started first (falling back to
/// insertion order for runs that haven't started executing yet, since
/// `started_at` is `NULL` until M6's execution loop stamps it).
#[allow(dead_code)]
pub fn list_for_agent(conn: &Connection, agent_id: &str) -> AppResult<Vec<AgentRun>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM agent_runs WHERE agent_id = ?1 ORDER BY started_at DESC, rowid DESC"
    ))?;
    let rows = stmt
        .query_map(params![agent_id], row_to_agent_run)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Inserts a `queued` run row for `agent_id` — this only records that a
/// worktree was created for `task_prompt` and is ready for execution to pick
/// up later; no model call is made here (that's M6).
pub fn insert_queued(conn: &Connection, agent_id: &str, task_prompt: &str, model_id: &str) -> AppResult<AgentRun> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO agent_runs (id, agent_id, workspace_id, task_prompt, model_id, status, stop_reason, error_message, iteration_count, total_input_tokens, total_output_tokens, started_at, completed_at)
         VALUES (?1, ?2, NULL, ?3, ?4, 'queued', NULL, NULL, 0, 0, 0, NULL, NULL)",
        params![id, agent_id, task_prompt, model_id],
    )?;
    Ok(AgentRun {
        id,
        agent_id: agent_id.to_string(),
        workspace_id: None,
        task_prompt: task_prompt.to_string(),
        model_id: model_id.to_string(),
        status: AgentRunStatus::Queued,
        stop_reason: None,
        error_message: None,
        iteration_count: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        started_at: None,
        completed_at: None,
    })
}

/// Links a run to the workspace created for it, once that workspace's own
/// row has been inserted (the workspace row itself references the run via
/// `agent_run_id`, so this is the other half of that link).
pub fn set_workspace_id(conn: &Connection, id: &str, workspace_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET workspace_id = ?2 WHERE id = ?1",
        params![id, workspace_id],
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
        conn.execute(
            "INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')",
            [],
        )
        .expect("insert agent");
        conn
    }

    #[test]
    fn insert_queued_then_get_by_id_round_trips() {
        let conn = setup_conn();
        let run = insert_queued(&conn, "a1", "Fix the bug", "claude-sonnet-5").expect("insert_queued");
        assert_eq!(run.status, AgentRunStatus::Queued);
        assert!(run.workspace_id.is_none());

        let fetched = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.task_prompt, "Fix the bug");
        assert_eq!(fetched.status, AgentRunStatus::Queued);
    }

    #[test]
    fn set_workspace_id_links_run_to_workspace() {
        let conn = setup_conn();
        let run = insert_queued(&conn, "a1", "Fix the bug", "claude-sonnet-5").expect("insert_queued");
        set_workspace_id(&conn, &run.id, "w1").expect("set_workspace_id");

        let fetched = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.workspace_id.as_deref(), Some("w1"));
    }

    #[test]
    fn list_for_agent_returns_inserted_run() {
        let conn = setup_conn();
        let run = insert_queued(&conn, "a1", "Fix the bug", "claude-sonnet-5").expect("insert_queued");
        let runs = list_for_agent(&conn, "a1").expect("list_for_agent");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, run.id);
    }
}

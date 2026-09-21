//! CRUD for the `agent_runs` table. `insert_queued` is called for real
//! starting M5 (one `queued` row per worktree created by
//! `commands::agent_commands::start_worktree_for_agent`) — nothing yet
//! transitions a run to `running`/`completed`/etc, since the actual model
//! call + tool loop that would do that lands in M6.

use chrono::Utc;
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

fn status_str(status: AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Queued => "queued",
        AgentRunStatus::Running => "running",
        AgentRunStatus::Completed => "completed",
        AgentRunStatus::Failed => "failed",
        AgentRunStatus::Stopped => "stopped",
    }
}

fn stop_reason_str(reason: AgentRunStopReason) -> &'static str {
    match reason {
        AgentRunStopReason::Completed => "completed",
        AgentRunStopReason::MaxIterations => "max_iterations",
        AgentRunStopReason::UserStopped => "user_stopped",
        AgentRunStopReason::Error => "error",
        AgentRunStopReason::TestFixBudgetExhausted => "test_fix_budget_exhausted",
    }
}

fn parse_stop_reason(s: Option<String>) -> Option<AgentRunStopReason> {
    match s.as_deref() {
        Some("completed") => Some(AgentRunStopReason::Completed),
        Some("max_iterations") => Some(AgentRunStopReason::MaxIterations),
        Some("user_stopped") => Some(AgentRunStopReason::UserStopped),
        Some("error") => Some(AgentRunStopReason::Error),
        Some("test_fix_budget_exhausted") => Some(AgentRunStopReason::TestFixBudgetExhausted),
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
        test_fix_attempts: row.get(11)?,
        started_at: row.get(12)?,
        completed_at: row.get(13)?,
    })
}

const SELECT_COLUMNS: &str = "id, agent_id, workspace_id, task_prompt, model_id, status, stop_reason, \
    error_message, iteration_count, total_input_tokens, total_output_tokens, test_fix_attempts, started_at, completed_at";

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
        "INSERT INTO agent_runs (id, agent_id, workspace_id, task_prompt, model_id, status, stop_reason, error_message, iteration_count, total_input_tokens, total_output_tokens, test_fix_attempts, started_at, completed_at)
         VALUES (?1, ?2, NULL, ?3, ?4, 'queued', NULL, NULL, 0, 0, 0, 0, NULL, NULL)",
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
        test_fix_attempts: 0,
        started_at: None,
        completed_at: None,
    })
}

/// Mirrors the in-memory `agent::test_fix::TestFixTracker`'s attempt count
/// onto this run's row — called by `agent::tool_loop` each time a
/// `run_tests` call changes that count (never incremented directly in SQL,
/// since the tracker itself is the source of truth for a single, sequential
/// run).
pub fn set_test_fix_attempts(conn: &Connection, id: &str, attempts: i64) -> AppResult<()> {
    conn.execute("UPDATE agent_runs SET test_fix_attempts = ?2 WHERE id = ?1", params![id, attempts])?;
    Ok(())
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

/// Transitions a `queued` run to `running` and stamps `started_at` — called
/// once by `agent::tool_loop::run_agent_loop` right before the first model
/// call, after the API key and workspace/settings lookups have all
/// succeeded.
pub fn mark_running(conn: &Connection, id: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET status = 'running', started_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

/// Terminal transition for a run — `completed`/`failed`/`stopped`, with the
/// matching `stop_reason` and (for `failed`, or a `report_completion` call
/// with `success: false`) a human-readable `error_message`. Stamps
/// `completed_at`. Called exactly once per run by
/// `agent::tool_loop::finish_run`.
pub fn mark_finished(
    conn: &Connection,
    id: &str,
    status: AgentRunStatus,
    stop_reason: Option<AgentRunStopReason>,
    error_message: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET status = ?2, stop_reason = ?3, error_message = ?4, completed_at = ?5 WHERE id = ?1",
        params![id, status_str(status), stop_reason.map(stop_reason_str), error_message, now],
    )?;
    Ok(())
}

/// Folds one model turn's token usage into the run's running totals and
/// bumps `iteration_count` — called once per turn from the tool loop, right
/// after a `StreamOutcome::Turn` comes back.
pub fn record_iteration_usage(conn: &Connection, id: &str, input_tokens: i64, output_tokens: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET iteration_count = iteration_count + 1, \
         total_input_tokens = total_input_tokens + ?2, total_output_tokens = total_output_tokens + ?3 \
         WHERE id = ?1",
        params![id, input_tokens, output_tokens],
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

    #[test]
    fn mark_running_then_mark_finished_round_trips() {
        let conn = setup_conn();
        let run = insert_queued(&conn, "a1", "Fix the bug", "claude-sonnet-5").expect("insert_queued");

        mark_running(&conn, &run.id).expect("mark_running");
        let running = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(running.status, AgentRunStatus::Running);
        assert!(running.started_at.is_some());

        mark_finished(&conn, &run.id, AgentRunStatus::Failed, Some(AgentRunStopReason::Error), Some("boom"))
            .expect("mark_finished");
        let finished = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(finished.status, AgentRunStatus::Failed);
        assert_eq!(finished.stop_reason, Some(AgentRunStopReason::Error));
        assert_eq!(finished.error_message.as_deref(), Some("boom"));
        assert!(finished.completed_at.is_some());
    }

    #[test]
    fn set_test_fix_attempts_updates_the_stored_count() {
        let conn = setup_conn();
        let run = insert_queued(&conn, "a1", "Fix the bug", "claude-sonnet-5").expect("insert_queued");
        assert_eq!(run.test_fix_attempts, 0);

        set_test_fix_attempts(&conn, &run.id, 2).expect("set_test_fix_attempts");
        let fetched = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.test_fix_attempts, 2);
    }

    #[test]
    fn record_iteration_usage_accumulates_across_calls() {
        let conn = setup_conn();
        let run = insert_queued(&conn, "a1", "Fix the bug", "claude-sonnet-5").expect("insert_queued");

        record_iteration_usage(&conn, &run.id, 100, 20).expect("record_iteration_usage");
        record_iteration_usage(&conn, &run.id, 50, 30).expect("record_iteration_usage");

        let fetched = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.iteration_count, 2);
        assert_eq!(fetched.total_input_tokens, 150);
        assert_eq!(fetched.total_output_tokens, 50);
    }
}

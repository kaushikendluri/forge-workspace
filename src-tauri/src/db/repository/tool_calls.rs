//! CRUD for the `tool_calls` table — one row per tool invocation the agent
//! loop (`agent::tool_loop`) makes during a run, inserted `running` and
//! updated to `success`/`error` once the tool finishes.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{ToolCall, ToolCallStatus};
use crate::error::AppResult;

fn status_str(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Running => "running",
        ToolCallStatus::Success => "success",
        ToolCallStatus::Error => "error",
    }
}

fn parse_status(s: &str) -> ToolCallStatus {
    match s {
        "success" => ToolCallStatus::Success,
        "error" => ToolCallStatus::Error,
        _ => ToolCallStatus::Running,
    }
}

fn row_to_tool_call(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCall> {
    let status: String = row.get(6)?;
    Ok(ToolCall {
        id: row.get(0)?,
        agent_run_id: row.get(1)?,
        sequence_number: row.get(2)?,
        tool_use_id: row.get(3)?,
        tool_name: row.get(4)?,
        input_json: row.get(5)?,
        status: parse_status(&status),
        output_json: row.get(7)?,
        error_message: row.get(8)?,
        started_at: row.get(9)?,
        completed_at: row.get(10)?,
        duration_ms: row.get(11)?,
    })
}

const SELECT_COLUMNS: &str = "id, agent_run_id, sequence_number, tool_use_id, tool_name, input_json, \
    status, output_json, error_message, started_at, completed_at, duration_ms";

/// All tool calls for a run, in call order.
pub fn list_for_run(conn: &Connection, agent_run_id: &str) -> AppResult<Vec<ToolCall>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM tool_calls WHERE agent_run_id = ?1 ORDER BY sequence_number ASC"
    ))?;
    let rows = stmt
        .query_map(params![agent_run_id], row_to_tool_call)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<ToolCall>> {
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM tool_calls WHERE id = ?1"), params![id], row_to_tool_call)
        .optional()
        .map_err(Into::into)
}

/// Inserts a new `running` tool call row.
pub fn insert_running(
    conn: &Connection,
    agent_run_id: &str,
    sequence_number: i64,
    tool_use_id: &str,
    tool_name: &str,
    input_json: &str,
) -> AppResult<ToolCall> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO tool_calls (id, agent_run_id, sequence_number, tool_use_id, tool_name, input_json, status, \
         output_json, error_message, started_at, completed_at, duration_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', NULL, NULL, ?7, NULL, NULL)",
        params![id, agent_run_id, sequence_number, tool_use_id, tool_name, input_json, now],
    )?;
    Ok(ToolCall {
        id,
        agent_run_id: agent_run_id.to_string(),
        sequence_number,
        tool_use_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        input_json: input_json.to_string(),
        status: ToolCallStatus::Running,
        output_json: None,
        error_message: None,
        started_at: now,
        completed_at: None,
        duration_ms: None,
    })
}

/// Marks a tool call finished (`success` or `error`), recording its output
/// text and duration.
pub fn complete(
    conn: &Connection,
    id: &str,
    status: ToolCallStatus,
    output_json: &str,
    error_message: Option<&str>,
    duration_ms: i64,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE tool_calls SET status = ?2, output_json = ?3, error_message = ?4, completed_at = ?5, duration_ms = ?6 \
         WHERE id = ?1",
        params![id, status_str(status), output_json, error_message, now, duration_ms],
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
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn insert_then_complete_round_trips() {
        let conn = setup_conn();
        let call = insert_running(&conn, "run1", 1, "toolu_1", "read_file", "{\"path\":\"a.txt\"}").unwrap();
        assert_eq!(call.status, ToolCallStatus::Running);

        complete(&conn, &call.id, ToolCallStatus::Success, "file contents", None, 12).unwrap();

        let fetched = get_by_id(&conn, &call.id).unwrap().unwrap();
        assert_eq!(fetched.status, ToolCallStatus::Success);
        assert_eq!(fetched.output_json.as_deref(), Some("file contents"));
        assert_eq!(fetched.duration_ms, Some(12));
    }

    #[test]
    fn list_for_run_orders_by_sequence_number() {
        let conn = setup_conn();
        insert_running(&conn, "run1", 2, "toolu_2", "read_file", "{}").unwrap();
        insert_running(&conn, "run1", 1, "toolu_1", "list_directory", "{}").unwrap();

        let calls = list_for_run(&conn, "run1").unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].sequence_number, 1);
        assert_eq!(calls[1].sequence_number, 2);
    }
}

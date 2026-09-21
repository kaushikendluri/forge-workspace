//! CRUD for the `activity_events` table — the run-level activity feed
//! (`agent::tool_loop` inserts one row per model turn and per tool call
//! lifecycle transition) that `AgentDetail`'s activity stream renders.

use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::db::models::{ActivityEvent, ActivityEventType};
use crate::error::AppResult;

fn event_type_str(event_type: ActivityEventType) -> &'static str {
    match event_type {
        ActivityEventType::RunStarted => "run_started",
        ActivityEventType::ModelMessage => "model_message",
        ActivityEventType::ToolCallStarted => "tool_call_started",
        ActivityEventType::ToolCallCompleted => "tool_call_completed",
        ActivityEventType::RunCompleted => "run_completed",
        ActivityEventType::RunStopped => "run_stopped",
        ActivityEventType::Error => "error",
        ActivityEventType::TestFixCycle => "test_fix_cycle",
    }
}

fn parse_event_type(s: &str) -> ActivityEventType {
    match s {
        "model_message" => ActivityEventType::ModelMessage,
        "tool_call_started" => ActivityEventType::ToolCallStarted,
        "tool_call_completed" => ActivityEventType::ToolCallCompleted,
        "run_completed" => ActivityEventType::RunCompleted,
        "run_stopped" => ActivityEventType::RunStopped,
        "error" => ActivityEventType::Error,
        "test_fix_cycle" => ActivityEventType::TestFixCycle,
        _ => ActivityEventType::RunStarted,
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityEvent> {
    let event_type: String = row.get(2)?;
    Ok(ActivityEvent {
        id: row.get(0)?,
        agent_run_id: row.get(1)?,
        event_type: parse_event_type(&event_type),
        tool_call_id: row.get(3)?,
        payload_json: row.get(4)?,
        created_at: row.get(5)?,
    })
}

const SELECT_COLUMNS: &str = "id, agent_run_id, event_type, tool_call_id, payload_json, created_at";

/// All activity for a run, oldest first (a chronological feed).
pub fn list_for_run(conn: &Connection, agent_run_id: &str) -> AppResult<Vec<ActivityEvent>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM activity_events WHERE agent_run_id = ?1 ORDER BY created_at ASC, rowid ASC"
    ))?;
    let rows = stmt
        .query_map(params![agent_run_id], row_to_event)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Inserts one activity event and returns it.
pub fn insert(
    conn: &Connection,
    agent_run_id: &str,
    tool_call_id: Option<&str>,
    event_type: ActivityEventType,
    payload_json: &str,
) -> AppResult<ActivityEvent> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO activity_events (id, agent_run_id, tool_call_id, event_type, payload_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, agent_run_id, tool_call_id, event_type_str(event_type), payload_json, now],
    )?;
    Ok(ActivityEvent {
        id,
        agent_run_id: Some(agent_run_id.to_string()),
        tool_call_id: tool_call_id.map(str::to_string),
        event_type,
        payload_json: payload_json.to_string(),
        created_at: now,
    })
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
    fn insert_then_list_round_trips_in_order() {
        let conn = setup_conn();
        insert(&conn, "run1", None, ActivityEventType::RunStarted, "{}").unwrap();
        insert(&conn, "run1", None, ActivityEventType::ModelMessage, "{\"text\":\"hi\"}").unwrap();

        let events = list_for_run(&conn, "run1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, ActivityEventType::RunStarted);
        assert_eq!(events[1].event_type, ActivityEventType::ModelMessage);
    }
}

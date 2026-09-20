//! CRUD for the `agent_messages` table (M11): structured agent-to-agent
//! communication within a mission. `insert` is called by the `send_message`
//! tool (`agent::tools`); `list_for_mission`/`list_for_agent_run` are the
//! read side for `commands::mission_commands::list_agent_messages` (Mission
//! Control's messages panel) and, potentially, a per-run view on
//! `AgentDetail.tsx`.

use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::db::models::AgentMessage;
use crate::error::AppResult;

const SELECT_COLUMNS: &str = "id, from_agent_run_id, to_agent_run_id, mission_id, subject, body, created_at";

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentMessage> {
    Ok(AgentMessage {
        id: row.get(0)?,
        from_agent_run_id: row.get(1)?,
        to_agent_run_id: row.get(2)?,
        mission_id: row.get(3)?,
        subject: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
    })
}

/// Inserts one agent-to-agent message and returns it. `to_agent_run_id` is
/// `None` for a mission-wide broadcast.
pub fn insert(
    conn: &Connection,
    from_agent_run_id: &str,
    to_agent_run_id: Option<&str>,
    mission_id: &str,
    subject: &str,
    body: &str,
) -> AppResult<AgentMessage> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_messages (id, from_agent_run_id, to_agent_run_id, mission_id, subject, body, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, from_agent_run_id, to_agent_run_id, mission_id, subject, body, now],
    )?;
    Ok(AgentMessage {
        id,
        from_agent_run_id: from_agent_run_id.to_string(),
        to_agent_run_id: to_agent_run_id.map(str::to_string),
        mission_id: mission_id.to_string(),
        subject: subject.to_string(),
        body: body.to_string(),
        created_at: now,
    })
}

/// Every message for `mission_id`, oldest first — a chronological
/// communication log for the whole mission, both broadcasts and
/// task-to-task messages.
pub fn list_for_mission(conn: &Connection, mission_id: &str) -> AppResult<Vec<AgentMessage>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM agent_messages WHERE mission_id = ?1 ORDER BY created_at ASC, rowid ASC"
    ))?;
    let rows = stmt.query_map(params![mission_id], row_to_message)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Every message either sent from or addressed to `agent_run_id`, oldest
/// first — for surfacing one run's own messages (e.g. on `AgentDetail.tsx`).
pub fn list_for_agent_run(conn: &Connection, agent_run_id: &str) -> AppResult<Vec<AgentMessage>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM agent_messages WHERE from_agent_run_id = ?1 OR to_agent_run_id = ?1 \
         ORDER BY created_at ASC, rowid ASC"
    ))?;
    let rows = stmt.query_map(params![agent_run_id], row_to_message)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
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
        conn.execute("INSERT INTO missions (id, project_id, objective) VALUES ('m1', 'p1', 'Ship it')", []).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run2', 'a1', 'do it too', 'claude-sonnet-5')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn insert_then_list_for_mission_round_trips_in_order() {
        let conn = setup_conn();
        insert(&conn, "run1", Some("run2"), "m1", "Handoff", "Schema is ready").expect("insert");
        insert(&conn, "run2", None, "m1", "Status", "Starting now").expect("insert");

        let messages = list_for_mission(&conn, "m1").expect("list_for_mission");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].subject, "Handoff");
        assert_eq!(messages[0].from_agent_run_id, "run1");
        assert_eq!(messages[0].to_agent_run_id.as_deref(), Some("run2"));
        assert_eq!(messages[1].subject, "Status");
        assert!(messages[1].to_agent_run_id.is_none(), "no recipient means a mission-wide broadcast");
    }

    #[test]
    fn list_for_agent_run_finds_messages_sent_or_received() {
        let conn = setup_conn();
        insert(&conn, "run1", Some("run2"), "m1", "To run2", "body").expect("insert");
        insert(&conn, "run2", Some("run1"), "m1", "To run1", "body").expect("insert");
        insert(&conn, "run1", None, "m1", "Broadcast", "body").expect("insert");

        let for_run1 = list_for_agent_run(&conn, "run1").expect("list_for_agent_run");
        assert_eq!(for_run1.len(), 3, "run1 sent two and received one");

        let for_run2 = list_for_agent_run(&conn, "run2").expect("list_for_agent_run");
        assert_eq!(for_run2.len(), 2, "run2 sent one and received one; the broadcast doesn't address it directly");
    }

    #[test]
    fn list_for_mission_scopes_to_that_mission_only() {
        let conn = setup_conn();
        conn.execute("INSERT INTO missions (id, project_id, objective) VALUES ('m2', 'p1', 'Other mission')", []).unwrap();
        insert(&conn, "run1", None, "m1", "For m1", "body").expect("insert");
        insert(&conn, "run2", None, "m2", "For m2", "body").expect("insert");

        let for_m1 = list_for_mission(&conn, "m1").expect("list_for_mission");
        assert_eq!(for_m1.len(), 1);
        assert_eq!(for_m1[0].subject, "For m1");
    }
}

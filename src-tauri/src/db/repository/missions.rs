//! CRUD for the `missions` table (M8): a plain-English objective, its
//! model-proposed task plan, and its human-approval lifecycle. Nothing here
//! advances a mission past `approved` — consuming an approved mission to
//! actually run its tasks is a later milestone.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{Mission, MissionStatus};
use crate::error::AppResult;

fn parse_status(s: &str) -> MissionStatus {
    match s {
        "plan_ready" => MissionStatus::PlanReady,
        "approved" => MissionStatus::Approved,
        "running" => MissionStatus::Running,
        "completed" => MissionStatus::Completed,
        "failed" => MissionStatus::Failed,
        _ => MissionStatus::Planning,
    }
}

const SELECT_COLUMNS: &str = "id, project_id, objective, status, plan_json, error_message, created_at, approved_at, completed_at";

fn row_to_mission(row: &rusqlite::Row<'_>) -> rusqlite::Result<Mission> {
    let status: String = row.get(3)?;
    Ok(Mission {
        id: row.get(0)?,
        project_id: row.get(1)?,
        objective: row.get(2)?,
        status: parse_status(&status),
        plan_json: row.get(4)?,
        error_message: row.get(5)?,
        created_at: row.get(6)?,
        approved_at: row.get(7)?,
        completed_at: row.get(8)?,
    })
}

/// Inserts a new mission row, status `planning` — the state it's in from
/// creation until the one planner call (`orchestrator::planner::propose_plan`)
/// resolves, at which point the caller moves it to `plan_ready` (via
/// `mark_plan_ready`) or `failed` (via `mark_failed`).
pub fn insert(conn: &Connection, project_id: &str, objective: &str) -> AppResult<Mission> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO missions (id, project_id, objective, status, plan_json, error_message, created_at, approved_at, completed_at)
         VALUES (?1, ?2, ?3, 'planning', NULL, NULL, ?4, NULL, NULL)",
        params![id, project_id, objective, now],
    )?;
    Ok(Mission {
        id,
        project_id: project_id.to_string(),
        objective: objective.to_string(),
        status: MissionStatus::Planning,
        plan_json: None,
        error_message: None,
        created_at: now,
        approved_at: None,
        completed_at: None,
    })
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<Mission>> {
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM missions WHERE id = ?1"), params![id], row_to_mission)
        .optional()
        .map_err(Into::into)
}

/// Missions for `project_id`, most recently created first.
pub fn list_for_project(conn: &Connection, project_id: &str) -> AppResult<Vec<Mission>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM missions WHERE project_id = ?1 ORDER BY created_at DESC"))?;
    let rows = stmt.query_map(params![project_id], row_to_mission)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Records a successfully planned mission: stores the raw plan JSON and
/// moves `status` to `plan_ready`, clearing any stale `error_message` from
/// an earlier failed attempt.
pub fn mark_plan_ready(conn: &Connection, id: &str, plan_json: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE missions SET status = 'plan_ready', plan_json = ?2, error_message = NULL WHERE id = ?1",
        params![id, plan_json],
    )?;
    Ok(())
}

/// Records a real planner failure (no API key configured, an Anthropic API
/// error, a malformed plan, ...) — `error_message` is always the genuine
/// error text, never a generic placeholder.
pub fn mark_failed(conn: &Connection, id: &str, error_message: &str) -> AppResult<()> {
    conn.execute("UPDATE missions SET status = 'failed', error_message = ?2 WHERE id = ?1", params![id, error_message])?;
    Ok(())
}

/// Records human approval of a `plan_ready` mission's plan. This is as far
/// as M8 takes a mission — nothing yet consumes `approved` to start
/// execution.
pub fn mark_approved(conn: &Connection, id: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute("UPDATE missions SET status = 'approved', approved_at = ?2 WHERE id = ?1", params![id, now])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')", []).expect("insert project");
        conn
    }

    #[test]
    fn insert_then_get_by_id_round_trips_as_planning() {
        let conn = setup_conn();
        let mission = insert(&conn, "p1", "Add dark mode").expect("insert");
        assert_eq!(mission.status, MissionStatus::Planning);
        assert!(mission.plan_json.is_none());

        let fetched = get_by_id(&conn, &mission.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.objective, "Add dark mode");
        assert_eq!(fetched.status, MissionStatus::Planning);
    }

    #[test]
    fn mark_plan_ready_stores_plan_and_clears_error() {
        let conn = setup_conn();
        let mission = insert(&conn, "p1", "Add dark mode").expect("insert");
        mark_failed(&conn, &mission.id, "transient error").expect("mark_failed");

        mark_plan_ready(&conn, &mission.id, "{\"tasks\":[]}").expect("mark_plan_ready");

        let fetched = get_by_id(&conn, &mission.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.status, MissionStatus::PlanReady);
        assert_eq!(fetched.plan_json.as_deref(), Some("{\"tasks\":[]}"));
        assert!(fetched.error_message.is_none(), "a successful plan should clear any earlier failure");
    }

    #[test]
    fn mark_failed_records_the_real_error() {
        let conn = setup_conn();
        let mission = insert(&conn, "p1", "Add dark mode").expect("insert");
        mark_failed(&conn, &mission.id, "No Anthropic API key is configured.").expect("mark_failed");

        let fetched = get_by_id(&conn, &mission.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.status, MissionStatus::Failed);
        assert_eq!(fetched.error_message.as_deref(), Some("No Anthropic API key is configured."));
    }

    #[test]
    fn mark_approved_stamps_approved_at() {
        let conn = setup_conn();
        let mission = insert(&conn, "p1", "Add dark mode").expect("insert");
        mark_plan_ready(&conn, &mission.id, "{\"tasks\":[]}").expect("mark_plan_ready");

        mark_approved(&conn, &mission.id).expect("mark_approved");

        let fetched = get_by_id(&conn, &mission.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.status, MissionStatus::Approved);
        assert!(fetched.approved_at.is_some());
    }

    #[test]
    fn list_for_project_most_recent_first() {
        let conn = setup_conn();
        let first = insert(&conn, "p1", "First objective").expect("insert 1");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = insert(&conn, "p1", "Second objective").expect("insert 2");

        let listed = list_for_project(&conn, "p1").expect("list_for_project");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, second.id);
        assert_eq!(listed[1].id, first.id);
    }
}

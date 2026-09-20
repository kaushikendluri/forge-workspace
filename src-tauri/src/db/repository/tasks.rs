//! CRUD for the `tasks` table. A task is either created directly (`create`,
//! status `todo`) or proposed by a mission's plan (`insert_for_mission`,
//! status `backlog` — see `orchestrator::planner` and
//! `commands::mission_commands`).

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{Task, TaskPriority, TaskStatus};
use crate::error::AppResult;

fn parse_status(s: &str) -> TaskStatus {
    match s {
        "backlog" => TaskStatus::Backlog,
        "in_progress" => TaskStatus::InProgress,
        "done" => TaskStatus::Done,
        "failed" => TaskStatus::Failed,
        "blocked" => TaskStatus::Blocked,
        "cancelled" => TaskStatus::Cancelled,
        _ => TaskStatus::Todo,
    }
}

fn status_str(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Backlog => "backlog",
        TaskStatus::Todo => "todo",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Done => "done",
        TaskStatus::Failed => "failed",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn parse_priority(s: &str) -> TaskPriority {
    match s {
        "low" => TaskPriority::Low,
        "high" => TaskPriority::High,
        _ => TaskPriority::Medium,
    }
}

pub fn priority_str(priority: TaskPriority) -> &'static str {
    match priority {
        TaskPriority::Low => "low",
        TaskPriority::Medium => "medium",
        TaskPriority::High => "high",
    }
}

const SELECT_COLUMNS: &str = "id, project_id, mission_id, title, description, status, priority, position, \
    depends_on_task_id, agent_type, agent_run_id, created_at, updated_at";

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status: String = row.get(5)?;
    let priority: String = row.get(6)?;
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        mission_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: parse_status(&status),
        priority: parse_priority(&priority),
        position: row.get(7)?,
        depends_on_task_id: row.get(8)?,
        agent_type: row.get(9)?,
        agent_run_id: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

/// Tasks for `project_id`, most recently created first — regardless of
/// which mission (if any) proposed them.
pub fn list_for_project(conn: &Connection, project_id: &str) -> AppResult<Vec<Task>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM tasks WHERE project_id = ?1 ORDER BY created_at DESC"))?;
    let rows = stmt.query_map(params![project_id], row_to_task)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Tasks proposed by `mission_id`'s plan, in the plan's own order.
pub fn list_for_mission(conn: &Connection, mission_id: &str) -> AppResult<Vec<Task>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM tasks WHERE mission_id = ?1 ORDER BY position ASC, created_at ASC"
    ))?;
    let rows = stmt.query_map(params![mission_id], row_to_task)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<Task>> {
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM tasks WHERE id = ?1"), params![id], row_to_task)
        .optional()
        .map_err(Into::into)
}

/// Creates a standalone task (status `todo`, no mission, default priority)
/// directly — as opposed to `insert_for_mission`, which is how a mission's
/// plan populates `tasks`.
pub fn create(conn: &Connection, project_id: &str, title: &str, description: Option<&str>) -> AppResult<Task> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO tasks (id, project_id, mission_id, title, description, status, priority, position, \
         depends_on_task_id, agent_type, agent_run_id, created_at, updated_at)
         VALUES (?1, ?2, NULL, ?3, ?4, 'todo', 'medium', 0, NULL, NULL, NULL, ?5, ?5)",
        params![id, project_id, title, description, now],
    )?;
    Ok(Task {
        id,
        project_id: project_id.to_string(),
        mission_id: None,
        title: title.to_string(),
        description: description.map(str::to_string),
        status: TaskStatus::Todo,
        priority: TaskPriority::Medium,
        position: 0,
        depends_on_task_id: None,
        agent_type: None,
        agent_run_id: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Inserts one task proposed by `mission_id`'s plan (status `backlog` —
/// planned, not yet started). `depends_on_task_id` starts `NULL`; the
/// caller (`commands::mission_commands::persist_plan`) fills it in with
/// `set_depends_on` once every task in the plan has a real row and id to
/// resolve dependency edges against.
#[allow(clippy::too_many_arguments)]
pub fn insert_for_mission(
    conn: &Connection,
    project_id: &str,
    mission_id: &str,
    title: &str,
    description: Option<&str>,
    agent_type: &str,
    priority: TaskPriority,
    position: i64,
) -> AppResult<Task> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO tasks (id, project_id, mission_id, title, description, status, priority, position, \
         depends_on_task_id, agent_type, agent_run_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'backlog', ?6, ?7, NULL, ?8, NULL, ?9, ?9)",
        params![id, project_id, mission_id, title, description, priority_str(priority), position, agent_type, now],
    )?;
    Ok(Task {
        id,
        project_id: project_id.to_string(),
        mission_id: Some(mission_id.to_string()),
        title: title.to_string(),
        description: description.map(str::to_string),
        status: TaskStatus::Backlog,
        priority,
        position,
        depends_on_task_id: None,
        agent_type: Some(agent_type.to_string()),
        agent_run_id: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Sets (or clears, with `None`) `task_id`'s `depends_on_task_id`.
pub fn set_depends_on(conn: &Connection, task_id: &str, depends_on_task_id: Option<&str>) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE tasks SET depends_on_task_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![task_id, depends_on_task_id, now],
    )?;
    Ok(())
}

pub fn update_status(conn: &Connection, id: &str, status: TaskStatus) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute("UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1", params![id, status_str(status), now])?;
    Ok(())
}

/// Links a task to the agent run the scheduler (M9) started for it — the
/// other half of `tasks.agent_run_id`, set once `start_worktree_for_agent`
/// has produced a real run to link (mirrors
/// `agent_runs::set_workspace_id`'s "link once the other row exists"
/// shape).
pub fn set_agent_run_id(conn: &Connection, id: &str, agent_run_id: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE tasks SET agent_run_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, agent_run_id, now],
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
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')", []).expect("insert project");
        conn.execute(
            "INSERT INTO missions (id, project_id, objective) VALUES ('m1', 'p1', 'Ship the thing')",
            [],
        )
        .expect("insert mission");
        conn
    }

    #[test]
    fn create_then_list_for_project_round_trips() {
        let conn = setup_conn();
        let task = create(&conn, "p1", "Write tests", Some("cover the happy path")).expect("create");
        assert_eq!(task.status, TaskStatus::Todo);
        assert_eq!(task.priority, TaskPriority::Medium);
        assert!(task.mission_id.is_none());

        let listed = list_for_project(&conn, "p1").expect("list_for_project");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, task.id);

        let fetched = get_by_id(&conn, &task.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.title, "Write tests");
    }

    #[test]
    fn insert_for_mission_defaults_to_backlog_status() {
        let conn = setup_conn();
        let task =
            insert_for_mission(&conn, "p1", "m1", "Design the schema", None, "database", TaskPriority::High, 0)
                .expect("insert_for_mission");
        assert_eq!(task.status, TaskStatus::Backlog);
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.mission_id.as_deref(), Some("m1"));
        assert_eq!(task.agent_type.as_deref(), Some("database"));

        let fetched = get_by_id(&conn, &task.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.status, TaskStatus::Backlog);
    }

    #[test]
    fn list_for_mission_orders_by_position() {
        let conn = setup_conn();
        let third = insert_for_mission(&conn, "p1", "m1", "Third", None, "qa", TaskPriority::Low, 2).expect("insert 3rd");
        let first =
            insert_for_mission(&conn, "p1", "m1", "First", None, "backend", TaskPriority::Medium, 0).expect("insert 1st");
        let second =
            insert_for_mission(&conn, "p1", "m1", "Second", None, "frontend", TaskPriority::Medium, 1).expect("insert 2nd");

        let listed = list_for_mission(&conn, "m1").expect("list_for_mission");
        assert_eq!(listed.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), vec![first.id, second.id, third.id]);
    }

    #[test]
    fn set_depends_on_resolves_a_real_foreign_key() {
        let conn = setup_conn();
        let upstream =
            insert_for_mission(&conn, "p1", "m1", "Provision the database", None, "database", TaskPriority::High, 0)
                .expect("insert upstream");
        let downstream = insert_for_mission(
            &conn,
            "p1",
            "m1",
            "Wire the backend to the database",
            None,
            "backend",
            TaskPriority::Medium,
            1,
        )
        .expect("insert downstream");

        set_depends_on(&conn, &downstream.id, Some(&upstream.id)).expect("set_depends_on");

        let fetched = get_by_id(&conn, &downstream.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.depends_on_task_id.as_deref(), Some(upstream.id.as_str()));
    }

    #[test]
    fn update_status_persists() {
        let conn = setup_conn();
        let task = create(&conn, "p1", "Ship it", None).expect("create");
        update_status(&conn, &task.id, TaskStatus::Done).expect("update_status");
        let fetched = get_by_id(&conn, &task.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.status, TaskStatus::Done);
    }

    #[test]
    fn update_status_round_trips_every_m9_scheduler_status() {
        let conn = setup_conn();
        let task = create(&conn, "p1", "Ship it", None).expect("create");
        for status in [TaskStatus::InProgress, TaskStatus::Failed, TaskStatus::Blocked, TaskStatus::Cancelled, TaskStatus::Done] {
            update_status(&conn, &task.id, status).expect("update_status");
            let fetched = get_by_id(&conn, &task.id).expect("get_by_id").expect("exists");
            assert_eq!(fetched.status, status);
        }
    }

    #[test]
    fn set_agent_run_id_links_task_to_its_run() {
        let conn = setup_conn();
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", [])
            .expect("insert repository");
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", [])
            .expect("insert agent");
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .expect("insert agent_run");

        let task = create(&conn, "p1", "Ship it", None).expect("create");
        assert!(task.agent_run_id.is_none());

        set_agent_run_id(&conn, &task.id, "run1").expect("set_agent_run_id");

        let fetched = get_by_id(&conn, &task.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.agent_run_id.as_deref(), Some("run1"));
    }
}

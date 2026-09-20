//! CRUD for the `test_runs` table (M12) — persisted history of manually
//! triggered test/lint/build runs. Backs `commands::testing_commands`.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{TestRun, TestRunKind, TestRunStatus};
use crate::error::AppResult;

fn kind_str(kind: TestRunKind) -> &'static str {
    match kind {
        TestRunKind::Test => "test",
        TestRunKind::Lint => "lint",
        TestRunKind::Build => "build",
    }
}

fn parse_kind(s: &str) -> TestRunKind {
    match s {
        "lint" => TestRunKind::Lint,
        "build" => TestRunKind::Build,
        _ => TestRunKind::Test,
    }
}

fn status_str(status: TestRunStatus) -> &'static str {
    match status {
        TestRunStatus::Running => "running",
        TestRunStatus::Success => "success",
        TestRunStatus::Failure => "failure",
    }
}

fn parse_status(s: &str) -> TestRunStatus {
    match s {
        "success" => TestRunStatus::Success,
        "failure" => TestRunStatus::Failure,
        _ => TestRunStatus::Running,
    }
}

fn row_to_test_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestRun> {
    let kind: String = row.get(2)?;
    let status: String = row.get(4)?;
    Ok(TestRun {
        id: row.get(0)?,
        project_id: row.get(1)?,
        kind: parse_kind(&kind),
        command: row.get(3)?,
        status: parse_status(&status),
        output: row.get(5)?,
        exit_code: row.get(6)?,
        started_at: row.get(7)?,
        completed_at: row.get(8)?,
    })
}

const SELECT_COLUMNS: &str = "id, project_id, kind, command, status, output, exit_code, started_at, completed_at";

/// Inserts a new `running` row for `project_id`/`kind`, about to execute
/// `command`. `run_test_suite` completes it via `complete` once the process
/// exits.
pub fn insert_running(conn: &Connection, project_id: &str, kind: TestRunKind, command: &str) -> AppResult<TestRun> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO test_runs (id, project_id, kind, command, status, output, exit_code, started_at, completed_at)
         VALUES (?1, ?2, ?3, ?4, 'running', NULL, NULL, ?5, NULL)",
        params![id, project_id, kind_str(kind), command, now],
    )?;
    Ok(TestRun {
        id,
        project_id: project_id.to_string(),
        kind,
        command: command.to_string(),
        status: TestRunStatus::Running,
        output: None,
        exit_code: None,
        started_at: now,
        completed_at: None,
    })
}

/// Terminal transition for a run — `success`/`failure`, with the process's
/// captured output and exit code. Stamps `completed_at`.
pub fn complete(conn: &Connection, id: &str, status: TestRunStatus, output: &str, exit_code: Option<i64>) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE test_runs SET status = ?2, output = ?3, exit_code = ?4, completed_at = ?5 WHERE id = ?1",
        params![id, status_str(status), output, exit_code, now],
    )?;
    Ok(())
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<TestRun>> {
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM test_runs WHERE id = ?1"), params![id], row_to_test_run)
        .optional()
        .map_err(Into::into)
}

/// `project_id`'s run history, most recent first, optionally narrowed to one
/// `kind` — capped at 50 rows (a manual-run history list, not an unbounded
/// audit log).
pub fn list_for_project(conn: &Connection, project_id: &str, kind: Option<TestRunKind>) -> AppResult<Vec<TestRun>> {
    // `rowid DESC` is the tiebreaker (matches `agent_runs_repo::list_for_agent`)
    // since `Utc::now().to_rfc3339()`'s variable fractional-second precision
    // doesn't always sort lexicographically the same as chronologically for
    // two rows inserted close together.
    let rows = match kind {
        Some(kind) => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLUMNS} FROM test_runs WHERE project_id = ?1 AND kind = ?2 ORDER BY started_at DESC, rowid DESC LIMIT 50"
            ))?;
            stmt.query_map(params![project_id, kind_str(kind)], row_to_test_run)?.collect::<Result<Vec<_>, _>>()?
        }
        None => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLUMNS} FROM test_runs WHERE project_id = ?1 ORDER BY started_at DESC, rowid DESC LIMIT 50"
            ))?;
            stmt.query_map(params![project_id], row_to_test_run)?.collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(rows)
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
    fn insert_running_then_get_by_id_round_trips() {
        let conn = setup_conn();
        let run = insert_running(&conn, "p1", TestRunKind::Test, "npm test").expect("insert_running");
        assert_eq!(run.status, TestRunStatus::Running);
        assert!(run.output.is_none());

        let fetched = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.command, "npm test");
        assert_eq!(fetched.kind, TestRunKind::Test);
    }

    #[test]
    fn complete_sets_terminal_status_and_output() {
        let conn = setup_conn();
        let run = insert_running(&conn, "p1", TestRunKind::Lint, "npm run lint").expect("insert_running");

        complete(&conn, &run.id, TestRunStatus::Failure, "2 problems found", Some(1)).expect("complete");

        let fetched = get_by_id(&conn, &run.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.status, TestRunStatus::Failure);
        assert_eq!(fetched.output.as_deref(), Some("2 problems found"));
        assert_eq!(fetched.exit_code, Some(1));
        assert!(fetched.completed_at.is_some());
    }

    #[test]
    fn list_for_project_filters_by_kind_and_orders_most_recent_first() {
        let conn = setup_conn();
        let first = insert_running(&conn, "p1", TestRunKind::Build, "cargo build").expect("insert 1");
        complete(&conn, &first.id, TestRunStatus::Success, "ok", Some(0)).expect("complete 1");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = insert_running(&conn, "p1", TestRunKind::Build, "cargo build").expect("insert 2");
        complete(&conn, &second.id, TestRunStatus::Success, "ok", Some(0)).expect("complete 2");
        insert_running(&conn, "p1", TestRunKind::Test, "cargo test").expect("insert test run");

        let builds = list_for_project(&conn, "p1", Some(TestRunKind::Build)).expect("list_for_project");
        assert_eq!(builds.len(), 2);
        assert_eq!(builds[0].id, second.id, "most recent build should come first");

        let all = list_for_project(&conn, "p1", None).expect("list_for_project all");
        assert_eq!(all.len(), 3);
    }
}

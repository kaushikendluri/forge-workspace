//! CRUD for the `project_brain` table (Phase 5 M17). Exactly one row per
//! project (`project_id UNIQUE`) — `upsert` replaces it in place rather than
//! inserting a new row each regeneration, since only the latest analysis is
//! ever useful (unlike `reviews`, which keeps every past verdict). Backs
//! `commands::brain_commands` and `brain::regenerate_brain`.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::ProjectBrain;
use crate::error::AppResult;

const SELECT_COLUMNS: &str =
    "id, project_id, stack_json, architecture_json, conventions_json, testing_json, important_files_json, generated_at, source_commit_sha";

fn row_to_brain(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectBrain> {
    Ok(ProjectBrain {
        id: row.get(0)?,
        project_id: row.get(1)?,
        stack_json: row.get(2)?,
        architecture_json: row.get(3)?,
        conventions_json: row.get(4)?,
        testing_json: row.get(5)?,
        important_files_json: row.get(6)?,
        generated_at: row.get(7)?,
        source_commit_sha: row.get(8)?,
    })
}

/// The project's brain, if one has ever been generated — `None` is a real,
/// honest "not yet analyzed" state, not an error.
pub fn get_by_project_id(conn: &Connection, project_id: &str) -> AppResult<Option<ProjectBrain>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM project_brain WHERE project_id = ?1"),
        params![project_id],
        row_to_brain,
    )
    .optional()
    .map_err(Into::into)
}

/// Inserts `project_id`'s first brain, or replaces its existing one in
/// place — `project_id`'s `UNIQUE` constraint plus `ON CONFLICT` is what
/// makes this a true upsert rather than ever accumulating duplicate rows.
/// `generated_at` is stamped fresh on every call (never passed in), so a
/// caller can't accidentally backdate it.
pub fn upsert(
    conn: &Connection,
    project_id: &str,
    stack_json: &str,
    architecture_json: &str,
    conventions_json: &str,
    testing_json: &str,
    important_files_json: &str,
    source_commit_sha: &str,
) -> AppResult<ProjectBrain> {
    let now = Utc::now().to_rfc3339();
    let existing_id = conn
        .query_row("SELECT id FROM project_brain WHERE project_id = ?1", params![project_id], |row| row.get::<_, String>(0))
        .optional()?;
    let id = existing_id.unwrap_or_else(|| Uuid::new_v4().to_string());

    conn.execute(
        "INSERT INTO project_brain \
         (id, project_id, stack_json, architecture_json, conventions_json, testing_json, important_files_json, generated_at, source_commit_sha) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT(project_id) DO UPDATE SET \
           stack_json = excluded.stack_json, \
           architecture_json = excluded.architecture_json, \
           conventions_json = excluded.conventions_json, \
           testing_json = excluded.testing_json, \
           important_files_json = excluded.important_files_json, \
           generated_at = excluded.generated_at, \
           source_commit_sha = excluded.source_commit_sha",
        params![id, project_id, stack_json, architecture_json, conventions_json, testing_json, important_files_json, now, source_commit_sha],
    )?;

    Ok(ProjectBrain {
        id,
        project_id: project_id.to_string(),
        stack_json: stack_json.to_string(),
        architecture_json: architecture_json.to_string(),
        conventions_json: conventions_json.to_string(),
        testing_json: testing_json.to_string(),
        important_files_json: important_files_json.to_string(),
        generated_at: now,
        source_commit_sha: source_commit_sha.to_string(),
    })
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
    fn get_by_project_id_is_none_before_any_generation() {
        let conn = setup_conn();
        assert!(get_by_project_id(&conn, "p1").expect("get_by_project_id").is_none());
    }

    #[test]
    fn upsert_then_get_round_trips() {
        let conn = setup_conn();
        let brain = upsert(&conn, "p1", r#"["React","TypeScript"]"#, "Frontend -> React", r#"["Strict TS"]"#, r#"["Vitest"]"#, "[]", "abc123")
            .expect("upsert");
        assert_eq!(brain.project_id, "p1");
        assert_eq!(brain.source_commit_sha, "abc123");

        let fetched = get_by_project_id(&conn, "p1").expect("get_by_project_id").expect("exists");
        assert_eq!(fetched.id, brain.id);
        assert_eq!(fetched.stack_json, r#"["React","TypeScript"]"#);
    }

    #[test]
    fn upsert_twice_replaces_in_place_rather_than_duplicating() {
        let conn = setup_conn();
        let first = upsert(&conn, "p1", r#"["React"]"#, "arch v1", "[]", "[]", "[]", "sha1").expect("first upsert");
        let second = upsert(&conn, "p1", r#"["React","Node"]"#, "arch v2", "[]", "[]", "[]", "sha2").expect("second upsert");

        assert_eq!(first.id, second.id, "the same project's row should be replaced in place, same id");

        let fetched = get_by_project_id(&conn, "p1").expect("get_by_project_id").expect("exists");
        assert_eq!(fetched.stack_json, r#"["React","Node"]"#, "the latest analysis should win");
        assert_eq!(fetched.source_commit_sha, "sha2");

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM project_brain WHERE project_id = 'p1'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "upsert must never accumulate a second row for the same project");
    }
}

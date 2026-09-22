//! CRUD for the `reviews` table (M14). `insert_pending` is called by
//! `agent::reviewer::run_review` before any model call is made (so a task
//! genuinely sitting in the Kanban board's Review column has a real row
//! backing it), `complete` transitions it to `passed`/`failed` once the
//! reviewer's structured verdict comes back, and `delete` removes a
//! `pending` row that never got to complete (an API error, no key
//! configured) rather than leaving a stuck row behind.

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use uuid::Uuid;

use crate::db::models::{Review, ReviewStatus};
use crate::error::AppResult;

fn status_str(status: ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Pending => "pending",
        ReviewStatus::Passed => "passed",
        ReviewStatus::Failed => "failed",
    }
}

fn parse_status(s: &str) -> ReviewStatus {
    match s {
        "passed" => ReviewStatus::Passed,
        "failed" => ReviewStatus::Failed,
        _ => ReviewStatus::Pending,
    }
}

const SELECT_COLUMNS: &str = "id, agent_run_id, score, findings_json, status, created_at";

fn row_to_review(row: &rusqlite::Row<'_>) -> rusqlite::Result<Review> {
    let status: String = row.get(4)?;
    Ok(Review {
        id: row.get(0)?,
        agent_run_id: row.get(1)?,
        score: row.get(2)?,
        findings_json: row.get(3)?,
        status: parse_status(&status),
        created_at: row.get(5)?,
    })
}

/// Inserts a `pending` placeholder row (`score: 0`, `findings_json: "[]"`)
/// for `agent_run_id`, right before the reviewer's real model calls start —
/// see this module's own docs for why.
pub fn insert_pending(conn: &Connection, agent_run_id: &str) -> AppResult<Review> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO reviews (id, agent_run_id, score, findings_json, status, created_at) \
         VALUES (?1, ?2, 0, '[]', 'pending', ?3)",
        params![id, agent_run_id, now],
    )?;
    Ok(Review {
        id,
        agent_run_id: agent_run_id.to_string(),
        score: 0,
        findings_json: "[]".to_string(),
        status: ReviewStatus::Pending,
        created_at: now,
    })
}

/// Terminal transition for a review — `passed`/`failed`, with the real
/// score and serialized findings the reviewer produced.
pub fn complete(conn: &Connection, id: &str, score: i64, findings_json: &str, status: ReviewStatus) -> AppResult<()> {
    conn.execute(
        "UPDATE reviews SET score = ?2, findings_json = ?3, status = ?4 WHERE id = ?1",
        params![id, score, findings_json, status_str(status)],
    )?;
    Ok(())
}

/// Removes a `pending` row that never completed (see `insert_pending`'s
/// docs) — best-effort cleanup, not a hard requirement of correctness (a
/// leftover `pending` row would just look like an in-progress review that
/// never was, rather than corrupt any other data).
pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM reviews WHERE id = ?1", params![id])?;
    Ok(())
}

/// The most recent review for `agent_run_id`, if any (a run can in
/// principle be reviewed more than once, e.g. a manual re-request — this is
/// always the latest attempt).
pub fn get_latest_for_run(conn: &Connection, agent_run_id: &str) -> AppResult<Option<Review>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM reviews WHERE agent_run_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1"),
        params![agent_run_id],
        row_to_review,
    )
    .optional()
    .map_err(Into::into)
}

/// Batched form of `get_latest_for_run`, one query for several run ids at
/// once — used by `commands::mission_commands::list_mission_board` to
/// derive both the board's Review column and each task's review-score badge
/// without one query per task. Missing/duplicate ids in `run_ids` are
/// harmless (deduped by the returned map's keys); an empty slice short
/// -circuits without touching the DB.
pub fn latest_reviews_for_runs(conn: &Connection, run_ids: &[String]) -> AppResult<HashMap<String, Review>> {
    let mut result = HashMap::new();
    if run_ids.is_empty() {
        return Ok(result);
    }
    let placeholders = vec!["?"; run_ids.len()].join(",");
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM reviews WHERE agent_run_id IN ({placeholders}) ORDER BY created_at DESC, rowid DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let bound: Vec<&dyn ToSql> = run_ids.iter().map(|s| s as &dyn ToSql).collect();
    let rows = stmt.query_map(bound.as_slice(), row_to_review)?.collect::<Result<Vec<_>, _>>()?;
    for review in rows {
        // Rows come back most-recent-first; `entry(..).or_insert(..)` keeps
        // only the first (i.e. latest) one seen per `agent_run_id`.
        result.entry(review.agent_run_id.clone()).or_insert(review);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')", []).expect("insert project");
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", [])
            .expect("insert repository");
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", [])
            .expect("insert agent");
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .expect("insert agent_run");
        conn
    }

    #[test]
    fn insert_pending_then_get_latest_round_trips() {
        let conn = setup_conn();
        let review = insert_pending(&conn, "run1").expect("insert_pending");
        assert_eq!(review.status, ReviewStatus::Pending);
        assert_eq!(review.score, 0);

        let fetched = get_latest_for_run(&conn, "run1").expect("get_latest_for_run").expect("exists");
        assert_eq!(fetched.id, review.id);
        assert_eq!(fetched.status, ReviewStatus::Pending);
    }

    #[test]
    fn complete_transitions_to_a_terminal_status_with_the_real_result() {
        let conn = setup_conn();
        let review = insert_pending(&conn, "run1").expect("insert_pending");

        complete(&conn, &review.id, 85, r#"[{"category":"style","severity":"low","summary":"nit"}]"#, ReviewStatus::Passed)
            .expect("complete");

        let fetched = get_latest_for_run(&conn, "run1").expect("get_latest_for_run").expect("exists");
        assert_eq!(fetched.status, ReviewStatus::Passed);
        assert_eq!(fetched.score, 85);
        assert!(fetched.findings_json.contains("nit"));
    }

    #[test]
    fn delete_removes_a_pending_row_that_never_completed() {
        let conn = setup_conn();
        let review = insert_pending(&conn, "run1").expect("insert_pending");
        delete(&conn, &review.id).expect("delete");
        assert!(get_latest_for_run(&conn, "run1").expect("get_latest_for_run").is_none());
    }

    #[test]
    fn get_latest_for_run_returns_the_most_recently_created_review() {
        let conn = setup_conn();
        let first = insert_pending(&conn, "run1").expect("insert 1");
        complete(&conn, &first.id, 40, "[]", ReviewStatus::Failed).expect("complete 1");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = insert_pending(&conn, "run1").expect("insert 2");
        complete(&conn, &second.id, 90, "[]", ReviewStatus::Passed).expect("complete 2");

        let latest = get_latest_for_run(&conn, "run1").expect("get_latest_for_run").expect("exists");
        assert_eq!(latest.id, second.id);
        assert_eq!(latest.score, 90);
    }

    #[test]
    fn latest_reviews_for_runs_batches_several_run_ids_at_once() {
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run2', 'a1', 'do it too', 'claude-sonnet-5')",
            [],
        )
        .expect("insert agent_run 2");

        let r1 = insert_pending(&conn, "run1").expect("insert run1 review");
        complete(&conn, &r1.id, 75, "[]", ReviewStatus::Passed).expect("complete run1 review");
        let r2 = insert_pending(&conn, "run2").expect("insert run2 review");
        complete(&conn, &r2.id, 50, "[]", ReviewStatus::Failed).expect("complete run2 review");

        let map = latest_reviews_for_runs(&conn, &["run1".to_string(), "run2".to_string(), "run-unknown".to_string()])
            .expect("latest_reviews_for_runs");
        assert_eq!(map.len(), 2);
        assert_eq!(map["run1"].score, 75);
        assert_eq!(map["run2"].status, ReviewStatus::Failed);
        assert!(!map.contains_key("run-unknown"));
    }

    #[test]
    fn latest_reviews_for_runs_is_empty_for_an_empty_input() {
        let conn = setup_conn();
        let map = latest_reviews_for_runs(&conn, &[]).expect("latest_reviews_for_runs");
        assert!(map.is_empty());
    }
}

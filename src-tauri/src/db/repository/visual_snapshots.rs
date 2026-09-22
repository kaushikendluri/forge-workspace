//! CRUD for the `visual_snapshots` table (M16). Rows are created by
//! `agent::tools::browser_screenshot_tool` right after a real screenshot is
//! captured and saved to disk; everything else here backs
//! `commands::visual_commands` (listing, and the Visual Regression panel's
//! Accept/Reject actions).

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{VisualSnapshot, VisualSnapshotKind};
use crate::error::{AppError, AppResult};

fn kind_str(kind: VisualSnapshotKind) -> &'static str {
    match kind {
        VisualSnapshotKind::Baseline => "baseline",
        VisualSnapshotKind::Comparison => "comparison",
    }
}

fn parse_kind(s: &str) -> VisualSnapshotKind {
    match s {
        "baseline" => VisualSnapshotKind::Baseline,
        _ => VisualSnapshotKind::Comparison,
    }
}

const SELECT_COLUMNS: &str = "id, agent_run_id, task_id, label, image_path, kind, flagged, created_at";

fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<VisualSnapshot> {
    let kind: String = row.get(5)?;
    Ok(VisualSnapshot {
        id: row.get(0)?,
        agent_run_id: row.get(1)?,
        task_id: row.get(2)?,
        label: row.get(3)?,
        image_path: row.get(4)?,
        kind: parse_kind(&kind),
        flagged: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

/// Whether `agent_run_id` already has a `baseline` row for `label` — decides
/// (in `agent::tools::browser_screenshot_tool`) whether the *next*
/// screenshot captured for that label becomes the baseline itself or a
/// comparison against the existing one.
pub fn has_baseline_for_label(conn: &Connection, agent_run_id: &str, label: &str) -> AppResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM visual_snapshots WHERE agent_run_id = ?1 AND label = ?2 AND kind = 'baseline'",
        params![agent_run_id, label],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Inserts one row for a screenshot that has already been captured and
/// saved to disk (`image_path` must already exist) — this never itself
/// touches the filesystem or a browser.
pub fn insert(
    conn: &Connection,
    agent_run_id: &str,
    task_id: Option<&str>,
    label: &str,
    image_path: &str,
    kind: VisualSnapshotKind,
) -> AppResult<VisualSnapshot> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO visual_snapshots (id, agent_run_id, task_id, label, image_path, kind, flagged) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
        params![id, agent_run_id, task_id, label, image_path, kind_str(kind)],
    )?;
    get_by_id(conn, &id)?.ok_or_else(|| AppError::Other("visual snapshot vanished immediately after insert".to_string()))
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<VisualSnapshot>> {
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM visual_snapshots WHERE id = ?1"), params![id], row_to_snapshot)
        .optional()
        .map_err(Into::into)
}

/// Every snapshot for `agent_run_id`, oldest first (so the frontend can show
/// each label's baseline before its later comparisons, in capture order).
pub fn list_for_run(conn: &Connection, agent_run_id: &str) -> AppResult<Vec<VisualSnapshot>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM visual_snapshots WHERE agent_run_id = ?1 ORDER BY created_at ASC"))?;
    let rows = stmt.query_map(params![agent_run_id], row_to_snapshot)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// "Accept": promotes `snapshot_id` to the new `baseline` for its
/// `(agent_run_id, label)` pair, demoting whatever was `baseline` before it
/// back to `comparison` — a plain DB update, nothing else (no file is
/// moved/deleted; the previous baseline's PNG stays on disk and is simply no
/// longer flagged as the baseline row).
pub fn accept_as_baseline(conn: &Connection, snapshot_id: &str) -> AppResult<()> {
    let snapshot =
        get_by_id(conn, snapshot_id)?.ok_or_else(|| AppError::NotFound(format!("visual snapshot {snapshot_id} not found")))?;
    conn.execute(
        "UPDATE visual_snapshots SET kind = 'comparison' WHERE agent_run_id = ?1 AND label = ?2 AND kind = 'baseline'",
        params![snapshot.agent_run_id, snapshot.label],
    )?;
    conn.execute("UPDATE visual_snapshots SET kind = 'baseline' WHERE id = ?1", params![snapshot_id])?;
    Ok(())
}

/// "Reject": marks `snapshot_id` flagged for a human's attention. Never
/// auto-reverts anything — see this module's own docs.
pub fn set_flagged(conn: &Connection, snapshot_id: &str, flagged: bool) -> AppResult<()> {
    let flagged_int: i64 = if flagged { 1 } else { 0 };
    conn.execute("UPDATE visual_snapshots SET flagged = ?2 WHERE id = ?1", params![snapshot_id, flagged_int])?;
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
    fn has_baseline_for_label_is_false_until_a_baseline_is_inserted() {
        let conn = setup_conn();
        assert!(!has_baseline_for_label(&conn, "run1", "home").unwrap());
        insert(&conn, "run1", None, "home", "/tmp/home-1.png", VisualSnapshotKind::Baseline).unwrap();
        assert!(has_baseline_for_label(&conn, "run1", "home").unwrap());
    }

    #[test]
    fn has_baseline_for_label_is_scoped_per_label() {
        let conn = setup_conn();
        insert(&conn, "run1", None, "home", "/tmp/home-1.png", VisualSnapshotKind::Baseline).unwrap();
        assert!(has_baseline_for_label(&conn, "run1", "home").unwrap());
        assert!(!has_baseline_for_label(&conn, "run1", "login").unwrap(), "a different label has no baseline of its own yet");
    }

    #[test]
    fn insert_then_get_by_id_round_trips() {
        let conn = setup_conn();
        let snapshot = insert(&conn, "run1", Some("t1"), "home", "/tmp/home-1.png", VisualSnapshotKind::Baseline).unwrap();
        assert_eq!(snapshot.kind, VisualSnapshotKind::Baseline);
        assert!(!snapshot.flagged);
        assert_eq!(snapshot.task_id.as_deref(), Some("t1"));

        let fetched = get_by_id(&conn, &snapshot.id).unwrap().expect("exists");
        assert_eq!(fetched.image_path, "/tmp/home-1.png");
        assert_eq!(fetched.label, "home");
    }

    #[test]
    fn list_for_run_returns_every_snapshot_oldest_first() {
        let conn = setup_conn();
        let first = insert(&conn, "run1", None, "home", "/tmp/1.png", VisualSnapshotKind::Baseline).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = insert(&conn, "run1", None, "home", "/tmp/2.png", VisualSnapshotKind::Comparison).unwrap();

        let listed = list_for_run(&conn, "run1").unwrap();
        assert_eq!(listed.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), vec![first.id, second.id]);
    }

    #[test]
    fn accept_as_baseline_promotes_the_comparison_and_demotes_the_old_baseline() {
        let conn = setup_conn();
        let baseline = insert(&conn, "run1", None, "home", "/tmp/1.png", VisualSnapshotKind::Baseline).unwrap();
        let comparison = insert(&conn, "run1", None, "home", "/tmp/2.png", VisualSnapshotKind::Comparison).unwrap();

        accept_as_baseline(&conn, &comparison.id).unwrap();

        let promoted = get_by_id(&conn, &comparison.id).unwrap().expect("exists");
        assert_eq!(promoted.kind, VisualSnapshotKind::Baseline);
        let demoted = get_by_id(&conn, &baseline.id).unwrap().expect("exists");
        assert_eq!(demoted.kind, VisualSnapshotKind::Comparison);
    }

    #[test]
    fn accept_as_baseline_only_demotes_the_baseline_for_the_same_label() {
        let conn = setup_conn();
        let home_baseline = insert(&conn, "run1", None, "home", "/tmp/1.png", VisualSnapshotKind::Baseline).unwrap();
        let login_baseline = insert(&conn, "run1", None, "login", "/tmp/2.png", VisualSnapshotKind::Baseline).unwrap();
        let home_comparison = insert(&conn, "run1", None, "home", "/tmp/3.png", VisualSnapshotKind::Comparison).unwrap();

        accept_as_baseline(&conn, &home_comparison.id).unwrap();

        assert_eq!(get_by_id(&conn, &home_baseline.id).unwrap().unwrap().kind, VisualSnapshotKind::Comparison);
        assert_eq!(
            get_by_id(&conn, &login_baseline.id).unwrap().unwrap().kind,
            VisualSnapshotKind::Baseline,
            "a different label's baseline must be untouched"
        );
    }

    #[test]
    fn accept_as_baseline_errors_clearly_for_an_unknown_id() {
        let conn = setup_conn();
        let err = accept_as_baseline(&conn, "does-not-exist").expect_err("should fail clearly");
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn set_flagged_persists() {
        let conn = setup_conn();
        let snapshot = insert(&conn, "run1", None, "home", "/tmp/1.png", VisualSnapshotKind::Comparison).unwrap();
        assert!(!get_by_id(&conn, &snapshot.id).unwrap().unwrap().flagged);

        set_flagged(&conn, &snapshot.id, true).unwrap();
        assert!(get_by_id(&conn, &snapshot.id).unwrap().unwrap().flagged);

        set_flagged(&conn, &snapshot.id, false).unwrap();
        assert!(!get_by_id(&conn, &snapshot.id).unwrap().unwrap().flagged);
    }
}

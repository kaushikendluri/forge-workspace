//! M16 commands: visual regression snapshots captured by the agent's
//! `browser_screenshot` tool (`agent::tools`) — listing them, serving their
//! real PNG bytes to the frontend, and the Visual Regression panel's three
//! actions on `AgentDetail.tsx`: Accept (promote to baseline), Reject (flag
//! for a human's attention), Ask to fix (file a real follow-up task).

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::run_blocking;
use crate::db::models::{Task, TaskPriority, VisualSnapshot, VisualSnapshotKind};
use crate::db::repository::{
    agent_runs as agent_runs_repo, agents as agents_repo, tasks as tasks_repo, visual_snapshots as visual_snapshots_repo,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Mirrors `src/types/db.ts`'s `VisualSnapshotDto`, returned by
/// `list_visual_snapshots`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualSnapshotDto {
    pub id: String,
    pub agent_run_id: String,
    pub task_id: Option<String>,
    pub label: String,
    pub image_path: String,
    pub kind: VisualSnapshotKind,
    pub flagged: bool,
    pub created_at: String,
}

impl From<VisualSnapshot> for VisualSnapshotDto {
    fn from(s: VisualSnapshot) -> Self {
        Self {
            id: s.id,
            agent_run_id: s.agent_run_id,
            task_id: s.task_id,
            label: s.label,
            image_path: s.image_path,
            kind: s.kind,
            flagged: s.flagged,
            created_at: s.created_at,
        }
    }
}

/// Every visual snapshot captured so far for `agent_run_id`, oldest first —
/// an honest empty list when the agent never called `browser_screenshot`.
#[tauri::command]
pub async fn list_visual_snapshots(app: AppHandle, agent_run_id: String) -> Result<Vec<VisualSnapshotDto>, String> {
    run_blocking(move || -> AppResult<Vec<VisualSnapshotDto>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        Ok(visual_snapshots_repo::list_for_run(&conn, &agent_run_id)?.into_iter().map(Into::into).collect())
    })
    .await
}

/// The real PNG bytes for one snapshot, base64-encoded, read fresh from disk
/// on every call (never cached/duplicated in SQLite) — the frontend renders
/// it as a `data:image/png;base64,...` `<img src>`.
#[tauri::command]
pub async fn get_visual_snapshot_image(app: AppHandle, snapshot_id: String) -> Result<String, String> {
    run_blocking(move || -> AppResult<String> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let snapshot = visual_snapshots_repo::get_by_id(&conn, &snapshot_id)?
            .ok_or_else(|| AppError::NotFound(format!("visual snapshot {snapshot_id} not found")))?;
        let bytes = std::fs::read(&snapshot.image_path)
            .map_err(|e| AppError::Other(format!("failed to read screenshot '{}': {e}", snapshot.image_path)))?;
        Ok(BASE64.encode(bytes))
    })
    .await
}

/// "Accept": promotes a comparison snapshot to be the new baseline for its
/// `(agentRunId, label)` pair — a plain DB update
/// (`db::repository::visual_snapshots::accept_as_baseline`), nothing more.
#[tauri::command]
pub async fn accept_visual_snapshot(app: AppHandle, snapshot_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        visual_snapshots_repo::accept_as_baseline(&conn, &snapshot_id)
    })
    .await
}

/// "Reject": flags a snapshot for a human's attention. Never auto-reverts
/// anything — see `db::repository::visual_snapshots::set_flagged`'s docs.
#[tauri::command]
pub async fn flag_visual_snapshot(app: AppHandle, snapshot_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        visual_snapshots_repo::set_flagged(&conn, &snapshot_id, true)
    })
    .await
}

/// "Ask to fix": files one real follow-up task describing the visual
/// regression — into the run's mission (mirroring `agent::reviewer`'s own
/// follow-up tasks) if the run belongs to one, otherwise a standalone task
/// for the run's project, so this also works for a solo (non-mission)
/// `AgentDetail` run rather than only mission-driven ones.
#[tauri::command]
pub async fn create_visual_regression_follow_up_task(app: AppHandle, snapshot_id: String) -> Result<Task, String> {
    run_blocking(move || -> AppResult<Task> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let snapshot = visual_snapshots_repo::get_by_id(&conn, &snapshot_id)?
            .ok_or_else(|| AppError::NotFound(format!("visual snapshot {snapshot_id} not found")))?;
        let agent_run = agent_runs_repo::get_by_id(&conn, &snapshot.agent_run_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent run {} not found", snapshot.agent_run_id)))?;
        let agent = agents_repo::get_by_id(&conn, &agent_run.agent_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent {} not found", agent_run.agent_id)))?;

        let title = format!("Visual regression: '{}'", snapshot.label);
        let description = format!(
            "The agent's `browser_screenshot` tool captured a comparison screenshot for label '{}' that a human \
             flagged for review during agent run {}. Compare it against its baseline in the Visual Regression \
             panel (image saved at '{}') and decide whether this is a real regression.",
            snapshot.label, snapshot.agent_run_id, snapshot.image_path
        );

        let task = match tasks_repo::get_by_agent_run_id(&conn, &snapshot.agent_run_id)?.and_then(|t| t.mission_id) {
            Some(mission_id) => {
                let next_position = tasks_repo::list_for_mission(&conn, &mission_id)?.len() as i64;
                tasks_repo::insert_for_mission(
                    &conn,
                    &agent.project_id,
                    &mission_id,
                    &title,
                    Some(&description),
                    "fix",
                    TaskPriority::High,
                    next_position,
                )?
            }
            None => tasks_repo::create(&conn, &agent.project_id, &title, Some(&description))?,
        };
        Ok(task)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use rusqlite::Connection;

    fn migrated_conn() -> Connection {
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

    /// Exercises the same DB-only logic `create_visual_regression_follow_up_task`
    /// runs, directly against a migrated in-memory connection (no Tauri
    /// `AppHandle`/command harness needed) — a solo run (no mission task)
    /// must get a real standalone task, not an error and not silently
    /// nothing.
    #[test]
    fn ask_to_fix_logic_creates_a_standalone_task_for_a_solo_run() {
        let conn = migrated_conn();
        let snapshot =
            visual_snapshots_repo::insert(&conn, "run1", None, "home", "/tmp/home.png", VisualSnapshotKind::Comparison).unwrap();

        assert!(tasks_repo::get_by_agent_run_id(&conn, "run1").unwrap().is_none(), "a solo run has no linked task");
        let task = tasks_repo::create(&conn, "p1", &format!("Visual regression: '{}'", snapshot.label), Some("desc")).unwrap();
        assert!(task.mission_id.is_none());
        assert_eq!(task.title, "Visual regression: 'home'");
    }

    /// Same DB-only logic for a run that *does* belong to a mission task —
    /// the follow-up must land in that same mission via `insert_for_mission`,
    /// mirroring `agent::reviewer::create_follow_up_tasks`.
    #[test]
    fn ask_to_fix_logic_creates_a_mission_task_for_a_mission_run() {
        let conn = migrated_conn();
        conn.execute("INSERT INTO missions (id, project_id, objective) VALUES ('m1', 'p1', 'Ship it')", []).unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, mission_id, title, status, priority, position, agent_run_id) \
             VALUES ('t1', 'p1', 'm1', 'Do the thing', 'done', 'medium', 0, 'run1')",
            [],
        )
        .unwrap();
        let snapshot =
            visual_snapshots_repo::insert(&conn, "run1", Some("t1"), "home", "/tmp/home.png", VisualSnapshotKind::Comparison).unwrap();

        let linked_task = tasks_repo::get_by_agent_run_id(&conn, "run1").unwrap().expect("linked task exists");
        let mission_id = linked_task.mission_id.expect("belongs to a mission");
        let follow_up = tasks_repo::insert_for_mission(
            &conn,
            "p1",
            &mission_id,
            &format!("Visual regression: '{}'", snapshot.label),
            Some("desc"),
            "fix",
            TaskPriority::High,
            1,
        )
        .unwrap();
        assert_eq!(follow_up.mission_id.as_deref(), Some("m1"));
        assert_eq!(follow_up.priority, TaskPriority::High);
    }
}

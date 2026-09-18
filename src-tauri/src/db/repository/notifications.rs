//! CRUD for the `notifications` table. `insert` is called from
//! `agent::tool_loop::finish_run` (M7) for every terminal agent-run
//! transition, so the bell icon in `TopBar.tsx` shows real, produced
//! notifications rather than a permanently-empty list.

use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::db::models::{Notification, NotificationType};
use crate::error::AppResult;

fn row_to_notification(row: &rusqlite::Row<'_>) -> rusqlite::Result<Notification> {
    let notification_type: String = row.get(3)?;
    let notification_type = match notification_type.as_str() {
        "agent_completed" => NotificationType::AgentCompleted,
        "agent_failed" => NotificationType::AgentFailed,
        _ => NotificationType::AgentStopped,
    };
    Ok(Notification {
        id: row.get(0)?,
        project_id: row.get(1)?,
        agent_run_id: row.get(2)?,
        notification_type,
        title: row.get(4)?,
        body: row.get(5)?,
        is_read: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

const SELECT_COLUMNS: &str = "id, project_id, agent_run_id, type, title, body, is_read, created_at";

fn type_str(t: NotificationType) -> &'static str {
    match t {
        NotificationType::AgentCompleted => "agent_completed",
        NotificationType::AgentFailed => "agent_failed",
        NotificationType::AgentStopped => "agent_stopped",
    }
}

/// Inserts a new notification row and returns it. Called from
/// `agent::tool_loop::finish_run` for every terminal (`completed`/`failed`/
/// `stopped`) agent-run transition.
#[allow(clippy::too_many_arguments)]
pub fn insert(
    conn: &Connection,
    project_id: Option<&str>,
    agent_run_id: Option<&str>,
    notification_type: NotificationType,
    title: &str,
    body: Option<&str>,
) -> AppResult<Notification> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notifications (id, project_id, agent_run_id, type, title, body, is_read, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![id, project_id, agent_run_id, type_str(notification_type), title, body, now],
    )?;
    Ok(Notification {
        id,
        project_id: project_id.map(str::to_string),
        agent_run_id: agent_run_id.map(str::to_string),
        notification_type,
        title: title.to_string(),
        body: body.map(str::to_string),
        is_read: false,
        created_at: now,
    })
}

/// Notifications for `project_id` (or every project, if `None`), most
/// recent first.
pub fn list_for_project(conn: &Connection, project_id: Option<&str>) -> AppResult<Vec<Notification>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM notifications
         WHERE ?1 IS NULL OR project_id = ?1
         ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![project_id], row_to_notification)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Marks a single notification as read.
pub fn mark_read(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("UPDATE notifications SET is_read = 1 WHERE id = ?1", params![id])?;
    Ok(())
}

/// Count of unread notifications for `project_id` (or every project, if
/// `None`).
pub fn unread_count(conn: &Connection, project_id: Option<&str>) -> AppResult<u32> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE is_read = 0 AND (?1 IS NULL OR project_id = ?1)",
        params![project_id],
        |row| row.get(0),
    )?;
    Ok(count as u32)
}

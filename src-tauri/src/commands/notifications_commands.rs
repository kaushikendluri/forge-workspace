//! Commands backing `TopBar.tsx`'s notification bell. Real notifications are
//! produced by `agent::tool_loop::finish_run` on every terminal agent-run
//! transition (M7); these commands are the read/write path the bell uses to
//! show them, plus a genuine "no notifications yet" empty state when there
//! are none.

use tauri::{AppHandle, Manager};

use crate::commands::run_blocking;
use crate::db::models::Notification;
use crate::db::repository::notifications as notifications_repo;
use crate::error::AppResult;
use crate::state::AppState;

/// Notifications for `project_id` (or every project, if `None`), most
/// recent first.
#[tauri::command]
pub async fn list_notifications(
    app: AppHandle,
    project_id: Option<String>,
) -> Result<Vec<Notification>, String> {
    run_blocking(move || -> AppResult<Vec<Notification>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        notifications_repo::list_for_project(&conn, project_id.as_deref())
    })
    .await
}

/// Marks a single notification as read.
#[tauri::command]
pub async fn mark_notification_read(app: AppHandle, id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        notifications_repo::mark_read(&conn, &id)
    })
    .await
}

/// Count of unread notifications for `project_id` (or every project, if
/// `None`) — backs the bell's unread badge.
#[tauri::command]
pub async fn unread_notification_count(
    app: AppHandle,
    project_id: Option<String>,
) -> Result<u32, String> {
    run_blocking(move || -> AppResult<u32> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        notifications_repo::unread_count(&conn, project_id.as_deref())
    })
    .await
}

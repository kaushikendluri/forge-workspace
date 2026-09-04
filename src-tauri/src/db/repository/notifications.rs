//! CRUD for the `notifications` table.

use rusqlite::Connection;

use crate::db::models::Notification;
use crate::error::AppResult;

pub fn list_unread(_conn: &Connection) -> AppResult<Vec<Notification>> {
    todo!("M5: SELECT * FROM notifications WHERE is_read = 0 ORDER BY created_at DESC")
}

pub fn mark_read(_conn: &Connection, _id: &str) -> AppResult<()> {
    todo!("M5: UPDATE notifications SET is_read = 1 WHERE id = ?1")
}

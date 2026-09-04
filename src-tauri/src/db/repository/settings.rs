//! CRUD for the `settings` key/value table. Backs `commands::settings_commands`.

use rusqlite::Connection;

use crate::db::models::Setting;
use crate::error::AppResult;

pub fn get(_conn: &Connection, _key: &str) -> AppResult<Option<Setting>> {
    todo!("M2: SELECT * FROM settings WHERE key = ?1")
}

pub fn set(_conn: &Connection, _key: &str, _value: &str) -> AppResult<()> {
    todo!("M2: INSERT INTO settings (...) VALUES (...) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = now()")
}

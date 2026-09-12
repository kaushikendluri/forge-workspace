//! CRUD for the `settings` key/value table. Backs `commands::settings_commands`.
//!
//! Values are stored as plain strings (the caller decides whether that
//! string is itself JSON, a number, etc.) — this module never interprets a
//! setting's meaning, just persists it. In particular, no API key or other
//! secret is ever written here: those live only in the OS keychain (see
//! `crate::secrets`).

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::models::Setting;
use crate::error::AppResult;

fn row_to_setting(row: &rusqlite::Row<'_>) -> rusqlite::Result<Setting> {
    Ok(Setting {
        key: row.get(0)?,
        value: row.get(1)?,
        updated_at: row.get(2)?,
    })
}

const SELECT_COLUMNS: &str = "key, value, updated_at";

/// The stored setting for `key`, or `None` if it's never been set.
pub fn get(conn: &Connection, key: &str) -> AppResult<Option<Setting>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM settings WHERE key = ?1"),
        params![key],
        row_to_setting,
    )
    .optional()
    .map_err(Into::into)
}

/// Inserts or updates the value for `key`.
pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now],
    )?;
    Ok(())
}

/// All settings currently stored, as a flat `key -> value` map.
pub fn get_all(conn: &Connection) -> AppResult<HashMap<String, String>> {
    let mut stmt = conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM settings"))?;
    let rows = stmt
        .query_map([], row_to_setting)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().map(|s| (s.key, s.value)).collect())
}

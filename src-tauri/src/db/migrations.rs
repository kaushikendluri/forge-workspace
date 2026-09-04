//! Applies SQL migrations from `../migrations/*.sql` in order, tracked in a
//! `schema_migrations` bookkeeping table so re-running the app doesn't
//! re-apply migrations that already ran.

use rusqlite::Connection;

use crate::error::AppResult;

/// (filename, embedded SQL) pairs, in application order. `include_str!` is
/// resolved at compile time relative to this file.
const MIGRATIONS: &[(&str, &str)] = &[("0001_init.sql", include_str!("../../migrations/0001_init.sql"))];

pub fn run_migrations(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        );",
    )?;

    for (name, sql) in MIGRATIONS {
        let already_applied: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )?;

        if already_applied {
            continue;
        }

        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (name) VALUES (?1)",
            [name],
        )?;
    }

    Ok(())
}

//! Applies SQL migrations from `../../migrations/*.sql`, embedded at compile
//! time via `include_str!`, using `rusqlite_migration`. It tracks the
//! applied schema version in its own bookkeeping table (`_rusqlite_migration_version`)
//! so re-running the app doesn't re-apply migrations that already ran.

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

use crate::error::{AppError, AppResult};

/// Migrations in application order. `include_str!` is resolved at compile
/// time relative to this file, so the embedded SQL is baked into the binary
/// — no runtime dependency on the `migrations/` directory existing.
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("../../migrations/0001_init.sql"))])
}

/// Brings `conn`'s schema up to the latest migration.
pub fn run_migrations(conn: &mut Connection) -> AppResult<()> {
    migrations()
        .to_latest(conn)
        .map_err(|e| AppError::Other(format!("migration failed: {e}")))
}

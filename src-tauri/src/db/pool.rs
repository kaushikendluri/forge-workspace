//! r2d2-pooled SQLite connections.

use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;

use crate::error::AppResult;

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;
pub type DbConnection = r2d2::PooledConnection<SqliteConnectionManager>;

/// Opens (creating if necessary) the SQLite database at `db_path` and
/// returns a connection pool. Foreign keys are enabled per-connection since
/// SQLite defaults them off.
///
/// `busy_timeout` (M10): before the parallel scheduler, at most one
/// connection at a time was ever likely to attempt a write (a single agent
/// run's own loop, or one M9 task at a time). WAL mode allows concurrent
/// readers alongside one writer, but SQLite's *default* `busy_timeout` is
/// `0` — a second writer that loses the race gets `SQLITE_BUSY` back
/// immediately instead of waiting. M10 genuinely has multiple pooled
/// connections (one per concurrently-running task's status
/// updates/tool-call/activity-event writes) racing to write at once, so a
/// real wait-and-retry window is needed rather than a bare hope that two
/// writers never land in the same instant. 5s is generous for this app's
/// tiny single-row writes; a real contention that outlasts 5s would indicate
/// something is actually stuck, not just contended.
pub fn create_pool(db_path: &Path) -> AppResult<DbPool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
    });
    let pool = r2d2::Pool::new(manager)?;
    Ok(pool)
}

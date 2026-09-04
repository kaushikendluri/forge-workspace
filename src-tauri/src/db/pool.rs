//! r2d2-pooled SQLite connections.

use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;

use crate::error::AppResult;

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;
pub type DbConnection = r2d2::PooledConnection<SqliteConnectionManager>;

/// Opens (creating if necessary) the SQLite database at `db_path` and
/// returns a connection pool. Foreign keys are enabled per-connection since
/// SQLite defaults them off.
pub fn create_pool(db_path: &Path) -> AppResult<DbPool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
    });
    let pool = r2d2::Pool::new(manager)?;
    Ok(pool)
}

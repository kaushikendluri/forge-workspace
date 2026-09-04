//! CRUD for the `model_configs` table (seeded with a Claude Sonnet 5 default
//! by `migrations/0001_init.sql`).

use rusqlite::Connection;

use crate::db::models::ModelConfig;
use crate::error::AppResult;

pub fn list(_conn: &Connection) -> AppResult<Vec<ModelConfig>> {
    todo!("M4: SELECT * FROM model_configs ORDER BY is_default DESC, display_name")
}

pub fn get_default(_conn: &Connection) -> AppResult<Option<ModelConfig>> {
    todo!("M4: SELECT * FROM model_configs WHERE is_default = 1 LIMIT 1")
}

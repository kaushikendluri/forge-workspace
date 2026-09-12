//! CRUD for the `model_configs` table (seeded with a Claude Sonnet 5 default
//! by `migrations/0001_init.sql`). Read-only for now — adding/editing model
//! configs is a later milestone (the Settings UI shows the seeded default
//! read-only).

use rusqlite::Connection;

use crate::db::models::ModelConfig;
use crate::error::AppResult;

fn row_to_model_config(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelConfig> {
    Ok(ModelConfig {
        id: row.get(0)?,
        provider: row.get(1)?,
        model_id: row.get(2)?,
        display_name: row.get(3)?,
        is_default: row.get::<_, i64>(4)? != 0,
        max_output_tokens: row.get(5)?,
        created_at: row.get(6)?,
    })
}

const SELECT_COLUMNS: &str =
    "id, provider, model_id, display_name, is_default, max_output_tokens, created_at";

pub fn list(conn: &Connection) -> AppResult<Vec<ModelConfig>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM model_configs ORDER BY is_default DESC, display_name"
    ))?;
    let rows = stmt
        .query_map([], row_to_model_config)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[allow(dead_code)]
pub fn get_default(conn: &Connection) -> AppResult<Option<ModelConfig>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        &format!("SELECT {SELECT_COLUMNS} FROM model_configs WHERE is_default = 1 LIMIT 1"),
        [],
        row_to_model_config,
    )
    .optional()
    .map_err(Into::into)
}

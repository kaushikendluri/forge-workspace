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

/// Phase 5 M19: looks up one model config by its id — used to resolve an
/// Agent Skill's `preferred_model_id` (itself just a reference to a row in
/// this table) into the real `model_id` string `agent_runs.model_id` needs,
/// at run-start time (`commands::agent_commands::start_worktree_for_agent`).
pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<ModelConfig>> {
    use rusqlite::OptionalExtension;
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM model_configs WHERE id = ?1"), rusqlite::params![id], row_to_model_config)
        .optional()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    #[test]
    fn get_by_id_finds_the_seeded_default_and_none_for_unknown() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");

        let found = get_by_id(&conn, "default-sonnet").expect("get_by_id").expect("seeded row exists");
        assert_eq!(found.model_id, "claude-sonnet-5");

        assert!(get_by_id(&conn, "nonexistent").expect("get_by_id").is_none());
    }
}

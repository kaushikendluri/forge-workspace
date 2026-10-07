//! CRUD for the `agent_skills` table (Phase 5 M19) — see
//! `migrations/0012_agent_skills.sql`'s own docs for the table's exact
//! shape. `tools_json` is validated here, at both [`insert`] and [`update`],
//! against the real tool catalogue (`agent::schema::all_tool_definitions`)
//! — an unknown tool name is rejected with a clear error naming it, rather
//! than silently accepted and only discovered to do nothing once a run
//! actually starts.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::agent::schema::all_tool_definitions;
use crate::db::models::AgentSkill;
use crate::error::{AppError, AppResult};

/// The sentinel `tools_json` element meaning "every tool in the real
/// catalogue, unrestricted" — exactly today's default (no skill at all)
/// behavior. Kept as a one-element JSON array (`["*"]`), not e.g. a bare
/// string or `null`, so `tools_json` always round-trips through
/// `serde_json::from_str::<Vec<String>>` the same way regardless of which
/// case it is.
pub const ALL_TOOLS_SENTINEL: &str = "*";

/// The `tools_json` a skill gets when none is explicitly given — "every
/// tool, unrestricted", matching today's plain-agent default exactly.
pub fn default_tools_json() -> String {
    serde_json::json!([ALL_TOOLS_SENTINEL]).to_string()
}

/// Parses `tools_json` into `None` (the `["*"]` sentinel was present —
/// "every tool, unrestricted") or `Some(names)` (an explicit, already
/// validated subset of real tool names — possibly empty, meaning "no tool
/// access at all"). Used both by [`validate_tools_json`] and by
/// `agent::tool_loop` to resolve the actual tool list for a skill-created
/// agent's run.
pub fn parse_tools_json(tools_json: &str) -> AppResult<Option<Vec<String>>> {
    let names: Vec<String> = serde_json::from_str(tools_json)
        .map_err(|e| AppError::InvalidInput(format!("tools_json must be a JSON array of strings: {e}")))?;
    if names.iter().any(|n| n == ALL_TOOLS_SENTINEL) {
        Ok(None)
    } else {
        Ok(Some(names))
    }
}

/// Rejects a candidate `tools_json` before it's ever written: must parse as
/// a JSON array of strings, and every name in it (other than the `"*"`
/// sentinel) must be a real tool `all_tool_definitions()` actually defines.
fn validate_tools_json(tools_json: &str) -> AppResult<()> {
    let Some(names) = parse_tools_json(tools_json)? else { return Ok(()) };
    let known: std::collections::HashSet<&str> = all_tool_definitions().iter().map(|d| d.name.as_str()).collect();
    for name in &names {
        if !known.contains(name.as_str()) {
            return Err(AppError::InvalidInput(format!(
                "unknown tool '{name}' in tools_json — must be a real tool from the catalogue, or \"{ALL_TOOLS_SENTINEL}\" for all tools"
            )));
        }
    }
    Ok(())
}

fn row_to_agent_skill(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSkill> {
    Ok(AgentSkill {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        instructions: row.get(3)?,
        tools_json: row.get(4)?,
        preferred_model_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

const SELECT_COLUMNS: &str =
    "id, name, description, instructions, tools_json, preferred_model_id, created_at, updated_at";

/// Every agent skill, alphabetically by name (a management list, not a
/// recency feed — unlike most other tables here).
pub fn list_all(conn: &Connection) -> AppResult<Vec<AgentSkill>> {
    let mut stmt = conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM agent_skills ORDER BY name COLLATE NOCASE"))?;
    let rows = stmt.query_map([], row_to_agent_skill)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<AgentSkill>> {
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM agent_skills WHERE id = ?1"), params![id], row_to_agent_skill)
        .optional()
        .map_err(Into::into)
}

/// Inserts a new agent skill. `tools_json` is validated against the real
/// tool catalogue first — nothing is written if it names an unknown tool.
pub fn insert(
    conn: &Connection,
    name: &str,
    description: Option<&str>,
    instructions: &str,
    tools_json: &str,
    preferred_model_id: Option<&str>,
) -> AppResult<AgentSkill> {
    validate_tools_json(tools_json)?;

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_skills (id, name, description, instructions, tools_json, preferred_model_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![id, name, description, instructions, tools_json, preferred_model_id, now],
    )?;
    Ok(AgentSkill {
        id,
        name: name.to_string(),
        description: description.map(str::to_string),
        instructions: instructions.to_string(),
        tools_json: tools_json.to_string(),
        preferred_model_id: preferred_model_id.map(str::to_string),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Full update of an existing skill's editable fields. Errors with
/// `AppError::NotFound` if `id` doesn't exist, and rejects an invalid
/// `tools_json` the same way [`insert`] does, before writing anything.
pub fn update(
    conn: &Connection,
    id: &str,
    name: &str,
    description: Option<&str>,
    instructions: &str,
    tools_json: &str,
    preferred_model_id: Option<&str>,
) -> AppResult<AgentSkill> {
    validate_tools_json(tools_json)?;

    let now = Utc::now().to_rfc3339();
    let rows_changed = conn.execute(
        "UPDATE agent_skills SET name = ?2, description = ?3, instructions = ?4, tools_json = ?5, \
         preferred_model_id = ?6, updated_at = ?7 WHERE id = ?1",
        params![id, name, description, instructions, tools_json, preferred_model_id, now],
    )?;
    if rows_changed == 0 {
        return Err(AppError::NotFound(format!("agent skill {id} not found")));
    }
    // Re-read rather than reconstructing by hand, so the returned row's
    // `created_at` (untouched by this update) is the real persisted value,
    // not a guess.
    get_by_id(conn, id)?.ok_or_else(|| AppError::NotFound(format!("agent skill {id} not found")))
}

/// Deletes a skill. Agents created from it keep their own copied
/// `system_prompt`; their `skill_id` is set to `NULL` by the FK's `ON
/// DELETE SET NULL` — this function doesn't need to touch `agents` itself.
pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let rows_changed = conn.execute("DELETE FROM agent_skills WHERE id = ?1", params![id])?;
    if rows_changed == 0 {
        return Err(AppError::NotFound(format!("agent skill {id} not found")));
    }
    Ok(())
}

/// A real duplicate: a fresh id, name suffixed `"Copy of <original name>"`,
/// everything else (description/instructions/tools_json/preferred_model_id)
/// copied verbatim from `id`. Errors with `AppError::NotFound` if `id`
/// doesn't exist.
pub fn duplicate(conn: &Connection, id: &str) -> AppResult<AgentSkill> {
    let original = get_by_id(conn, id)?.ok_or_else(|| AppError::NotFound(format!("agent skill {id} not found")))?;
    let new_name = format!("Copy of {}", original.name);
    insert(
        conn,
        &new_name,
        original.description.as_deref(),
        &original.instructions,
        &original.tools_json,
        original.preferred_model_id.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn
    }

    #[test]
    fn insert_then_get_round_trips_with_default_all_tools_sentinel() {
        let conn = setup_conn();
        let skill = insert(&conn, "Senior React Engineer", Some("Frontend specialist"), "Write idiomatic React/TS.", &default_tools_json(), None)
            .expect("insert");
        assert_eq!(skill.tools_json, "[\"*\"]");

        let fetched = get_by_id(&conn, &skill.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.name, "Senior React Engineer");
        assert_eq!(fetched.instructions, "Write idiomatic React/TS.");
        assert_eq!(parse_tools_json(&fetched.tools_json).unwrap(), None);
    }

    #[test]
    fn insert_rejects_unknown_tool_name() {
        let conn = setup_conn();
        let err = insert(&conn, "Bad Skill", None, "instructions", "[\"read_file\", \"nonexistent_tool\"]", None)
            .expect_err("must reject an unknown tool name");
        assert!(err.to_string().contains("nonexistent_tool"));

        // Nothing was written.
        assert!(list_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn insert_accepts_a_real_restricted_subset() {
        let conn = setup_conn();
        let skill = insert(&conn, "Reviewer-ish", None, "instructions", "[\"read_file\", \"git_status\"]", None).expect("insert");
        let names = parse_tools_json(&skill.tools_json).unwrap().expect("explicit subset, not the all-tools sentinel");
        assert_eq!(names, vec!["read_file".to_string(), "git_status".to_string()]);
    }

    #[test]
    fn insert_accepts_empty_tools_array_as_no_tools() {
        let conn = setup_conn();
        let skill = insert(&conn, "No Tools", None, "instructions", "[]", None).expect("insert");
        let names = parse_tools_json(&skill.tools_json).unwrap().expect("empty is an explicit subset, not the sentinel");
        assert!(names.is_empty());
    }

    #[test]
    fn list_all_orders_alphabetically_by_name() {
        let conn = setup_conn();
        insert(&conn, "Zebra", None, "i", &default_tools_json(), None).unwrap();
        insert(&conn, "Alpha", None, "i", &default_tools_json(), None).unwrap();
        let names: Vec<String> = list_all(&conn).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["Alpha".to_string(), "Zebra".to_string()]);
    }

    #[test]
    fn update_changes_fields_and_rejects_unknown_tool() {
        let conn = setup_conn();
        let skill = insert(&conn, "Original", None, "orig instructions", &default_tools_json(), None).unwrap();

        let updated = update(&conn, &skill.id, "Renamed", Some("new desc"), "new instructions", "[\"read_file\"]", None)
            .expect("update");
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.instructions, "new instructions");

        let fetched = get_by_id(&conn, &skill.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Renamed");
        assert_eq!(fetched.description, Some("new desc".to_string()));

        let err = update(&conn, &skill.id, "Renamed", None, "x", "[\"totally_fake\"]", None).expect_err("must reject");
        assert!(err.to_string().contains("totally_fake"));
    }

    #[test]
    fn update_unknown_id_is_not_found() {
        let conn = setup_conn();
        let err = update(&conn, "nonexistent", "x", None, "x", &default_tools_json(), None).expect_err("must error");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn delete_removes_the_row() {
        let conn = setup_conn();
        let skill = insert(&conn, "Temp", None, "i", &default_tools_json(), None).unwrap();
        delete(&conn, &skill.id).expect("delete");
        assert!(get_by_id(&conn, &skill.id).unwrap().is_none());
    }

    #[test]
    fn delete_unknown_id_is_not_found() {
        let conn = setup_conn();
        let err = delete(&conn, "nonexistent").expect_err("must error");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn duplicate_copies_every_field_with_a_fresh_id_and_suffixed_name() {
        let conn = setup_conn();
        let original =
            insert(&conn, "Security Auditor", Some("desc"), "Find vulnerabilities.", "[\"read_file\", \"search_code\"]", None)
                .unwrap();

        let copy = duplicate(&conn, &original.id).expect("duplicate");
        assert_ne!(copy.id, original.id);
        assert_eq!(copy.name, "Copy of Security Auditor");
        assert_eq!(copy.description, original.description);
        assert_eq!(copy.instructions, original.instructions);
        assert_eq!(copy.tools_json, original.tools_json);

        // Both rows genuinely exist now, independently.
        assert_eq!(list_all(&conn).unwrap().len(), 2);
    }

    #[test]
    fn duplicate_unknown_id_is_not_found() {
        let conn = setup_conn();
        let err = duplicate(&conn, "nonexistent").expect_err("must error");
        assert!(matches!(err, AppError::NotFound(_)));
    }
}

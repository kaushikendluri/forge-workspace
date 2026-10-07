//! Commands for Phase 5 M19's Agent Skills: plain CRUD over the
//! `agent_skills` table (`db::repository::agent_skills`), plus
//! `list_available_tools` — the real tool catalogue
//! (`agent::schema::all_tool_definitions`), exposed so the frontend's tool
//! checklist is built from the same source of truth the backend validates
//! `tools_json` against, rather than a hand-maintained duplicate list.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::agent::schema::all_tool_definitions;
use crate::commands::run_blocking;
use crate::db::models::AgentSkill;
use crate::db::repository::agent_skills as agent_skills_repo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// One entry of the real tool catalogue, for the frontend's tool checklist.
/// Mirrors `agent::anthropic_client::ToolDefinition`'s `name`/`description`
/// (never `input_schema` — the frontend only needs to list/select tools by
/// name, not build the schema itself).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogueEntryDto {
    pub name: String,
    pub description: String,
}

/// The real tool catalogue a skill's `tools_json` can restrict to — the
/// same list `agent::schema::all_tool_definitions` builds for an actual
/// agent run, so the frontend's checklist can never drift from what the
/// backend will actually validate/offer.
#[tauri::command]
pub async fn list_available_tools() -> Result<Vec<ToolCatalogueEntryDto>, String> {
    Ok(all_tool_definitions().into_iter().map(|d| ToolCatalogueEntryDto { name: d.name, description: d.description }).collect())
}

/// All agent skills, alphabetically by name.
#[tauri::command]
pub async fn list_agent_skills(app: AppHandle) -> Result<Vec<AgentSkill>, String> {
    run_blocking(move || -> AppResult<Vec<AgentSkill>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_skills_repo::list_all(&conn)
    })
    .await
}

/// One agent skill by id, or `null` if it doesn't exist.
#[tauri::command]
pub async fn get_agent_skill(app: AppHandle, skill_id: String) -> Result<Option<AgentSkill>, String> {
    run_blocking(move || -> AppResult<Option<AgentSkill>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_skills_repo::get_by_id(&conn, &skill_id)
    })
    .await
}

/// Creates a new agent skill. `tools_json` must be a JSON array of real
/// tool names (or the `"*"` sentinel for "all tools") — `None` defaults to
/// the "all tools" sentinel, matching today's unrestricted agent behavior.
/// Rejects an unknown tool name with a clear error before writing anything.
#[tauri::command]
pub async fn create_agent_skill(
    app: AppHandle,
    name: String,
    description: Option<String>,
    instructions: String,
    tools_json: Option<String>,
    preferred_model_id: Option<String>,
) -> Result<AgentSkill, String> {
    run_blocking(move || -> AppResult<AgentSkill> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;

        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::InvalidInput("skill name cannot be empty".to_string()));
        }
        let instructions = instructions.trim();
        if instructions.is_empty() {
            return Err(AppError::InvalidInput("skill instructions cannot be empty".to_string()));
        }
        let tools_json = tools_json.unwrap_or_else(agent_skills_repo::default_tools_json);
        let description = description.as_deref().map(str::trim).filter(|s| !s.is_empty());

        agent_skills_repo::insert(&conn, name, description, instructions, &tools_json, preferred_model_id.as_deref())
    })
    .await
}

/// Full update of an existing skill's editable fields. Same validation as
/// [`create_agent_skill`]; errors if `skill_id` doesn't exist.
#[tauri::command]
pub async fn update_agent_skill(
    app: AppHandle,
    skill_id: String,
    name: String,
    description: Option<String>,
    instructions: String,
    tools_json: Option<String>,
    preferred_model_id: Option<String>,
) -> Result<AgentSkill, String> {
    run_blocking(move || -> AppResult<AgentSkill> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;

        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::InvalidInput("skill name cannot be empty".to_string()));
        }
        let instructions = instructions.trim();
        if instructions.is_empty() {
            return Err(AppError::InvalidInput("skill instructions cannot be empty".to_string()));
        }
        let tools_json = tools_json.unwrap_or_else(agent_skills_repo::default_tools_json);
        let description = description.as_deref().map(str::trim).filter(|s| !s.is_empty());

        agent_skills_repo::update(&conn, &skill_id, name, description, instructions, &tools_json, preferred_model_id.as_deref())
    })
    .await
}

/// Deletes an agent skill. Agents previously created from it keep their own
/// already-copied `system_prompt`; their `skill_id` becomes `NULL` (the
/// `agents.skill_id` foreign key's `ON DELETE SET NULL`) — this never
/// cascades into deleting agents.
#[tauri::command]
pub async fn delete_agent_skill(app: AppHandle, skill_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_skills_repo::delete(&conn, &skill_id)
    })
    .await
}

/// A real duplicate: a fresh id, name suffixed `"Copy of <original name>"`,
/// everything else copied verbatim from `skill_id`.
#[tauri::command]
pub async fn duplicate_agent_skill(app: AppHandle, skill_id: String) -> Result<AgentSkill, String> {
    run_blocking(move || -> AppResult<AgentSkill> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_skills_repo::duplicate(&conn, &skill_id)
    })
    .await
}

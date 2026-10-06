//! Phase 5 M17 commands: reading and explicitly regenerating a project's
//! Project Brain. See `brain`'s own module docs for what generation
//! actually involves (file-scanning context + one structured Anthropic
//! call) and the regeneration policy `project_commands::open_project`/
//! `init_project` apply automatically in the background
//! (`brain::maybe_auto_regenerate`) — this module is only the read +
//! explicit-refresh surface the frontend calls directly.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::brain;
use crate::commands::run_blocking;
use crate::db::models::ProjectBrain;
use crate::db::repository::project_brain as project_brain_repo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Mirrors `brain::BrainImportantFile` — kept as a separate (identical)
/// type at the command boundary the same way every other `*Dto` here does,
/// rather than exposing an internal module's type directly over IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBrainImportantFileDto {
    pub path: String,
    pub why: String,
}

/// The shape the Project Brain panel actually renders — every `*_json`
/// column of the stored `project_brain` row parsed back into real typed
/// values. Returned wrapped in `Option` by `get_project_brain`: `None` is
/// the honest "not yet analyzed" state, never a fabricated placeholder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBrainDto {
    pub id: String,
    pub project_id: String,
    pub stack: Vec<String>,
    pub architecture: String,
    pub conventions: Vec<String>,
    pub testing: Vec<String>,
    pub important_files: Vec<ProjectBrainImportantFileDto>,
    pub generated_at: String,
    pub source_commit_sha: String,
}

fn to_dto(row: ProjectBrain) -> AppResult<ProjectBrainDto> {
    let stack: Vec<String> =
        serde_json::from_str(&row.stack_json).map_err(|e| AppError::Other(format!("failed to parse stored brain stack: {e}")))?;
    let architecture: String = serde_json::from_str(&row.architecture_json)
        .map_err(|e| AppError::Other(format!("failed to parse stored brain architecture: {e}")))?;
    let conventions: Vec<String> = serde_json::from_str(&row.conventions_json)
        .map_err(|e| AppError::Other(format!("failed to parse stored brain conventions: {e}")))?;
    let testing: Vec<String> =
        serde_json::from_str(&row.testing_json).map_err(|e| AppError::Other(format!("failed to parse stored brain testing: {e}")))?;
    let important_files: Vec<ProjectBrainImportantFileDto> = serde_json::from_str(&row.important_files_json)
        .map_err(|e| AppError::Other(format!("failed to parse stored brain important_files: {e}")))?;

    Ok(ProjectBrainDto {
        id: row.id,
        project_id: row.project_id,
        stack,
        architecture,
        conventions,
        testing,
        important_files,
        generated_at: row.generated_at,
        source_commit_sha: row.source_commit_sha,
    })
}

/// `project_id`'s Project Brain, if one has ever been generated — `None` is
/// the honest "not yet analyzed" state, never a fabricated placeholder.
#[tauri::command]
pub async fn get_project_brain(app: AppHandle, project_id: String) -> Result<Option<ProjectBrainDto>, String> {
    run_blocking(move || -> AppResult<Option<ProjectBrainDto>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        match project_brain_repo::get_by_project_id(&conn, &project_id)? {
            Some(row) => Ok(Some(to_dto(row)?)),
            None => Ok(None),
        }
    })
    .await
}

/// Explicit, user-requested "Regenerate" — always runs a real analysis
/// (`brain::regenerate_brain`) regardless of the background auto-refresh
/// policy, and surfaces any real failure (no API key configured, an API
/// error, no repository registered) rather than swallowing it the way the
/// fire-and-forget background check (`brain::maybe_auto_regenerate`) does.
#[tauri::command]
pub async fn regenerate_project_brain(app: AppHandle, project_id: String) -> Result<ProjectBrainDto, String> {
    let row = brain::regenerate_brain(&app, &project_id).await.map_err(|e| e.to_string())?;
    to_dto(row).map_err(|e| e.to_string())
}

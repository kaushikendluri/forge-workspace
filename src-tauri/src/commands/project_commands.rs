//! Commands backing `src/stores/useProjectStore.ts` and the
//! Dashboard/Projects routes. See `src/lib/tauri.ts`'s `CommandMap` for the
//! matching frontend-side request/response types.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::commands::run_blocking;
use crate::db::models::{Agent, Project, Repository, Task};
use crate::db::repository::{projects as projects_repo, repositories as repositories_repo};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// A project row joined with its (Phase 1: single) repository — the shape
/// the frontend actually renders on the Dashboard/Projects routes and the
/// project workspace shell. Field names are camelCase over the IPC bridge
/// to match `src/types/db.ts`'s `ProjectDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
    pub repository_id: String,
    pub root_path: String,
    pub remote_url: Option<String>,
    pub default_branch: String,
    pub vcs_type: String,
}

fn to_dto(project: Project, repository: Repository) -> ProjectDto {
    ProjectDto {
        id: project.id,
        name: project.name,
        description: project.description,
        created_at: project.created_at,
        updated_at: project.updated_at,
        last_opened_at: project.last_opened_at,
        repository_id: repository.id,
        root_path: repository.root_path,
        remote_url: repository.remote_url,
        default_branch: repository.default_branch,
        vcs_type: repository.vcs_type,
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

/// Shared implementation for `open_project` and `init_project`: registers
/// `path` as a project's repository root if it isn't one already (bumping
/// `last_opened_at` if it is), and returns the combined DTO. `path` must
/// already exist and be a git repository by the time this runs — callers
/// are responsible for `git init`-ing first if that's what they want.
fn open_or_register(state: &AppState, path: &str, explicit_name: Option<&str>) -> AppResult<ProjectDto> {
    let root = PathBuf::from(path);
    if !root.is_dir() {
        return Err(AppError::InvalidInput(format!("'{path}' is not a directory")));
    }
    if !state.git_service.is_git_repository(&root) {
        return Err(AppError::InvalidInput(format!(
            "'{path}' is not a git repository"
        )));
    }

    // Canonicalize so the same folder opened via different (relative,
    // symlinked, differently-cased-on-Windows) paths maps to one repository
    // row instead of silently duplicating it.
    let canonical = root.canonicalize().unwrap_or(root);
    let root_path = canonical.to_string_lossy().to_string();

    let conn = state.db.get()?;

    if let Some(repository) = repositories_repo::get_by_root_path(&conn, &root_path)? {
        projects_repo::update_last_opened(&conn, &repository.project_id)?;
        let project = projects_repo::get_by_id(&conn, &repository.project_id)?.ok_or_else(|| {
            AppError::NotFound(format!("project {} not found", repository.project_id))
        })?;
        return Ok(to_dto(project, repository));
    }

    let default_branch = state
        .git_service
        .current_branch(&canonical)
        .ok()
        .flatten()
        .unwrap_or_else(|| "main".to_string());

    let name = explicit_name
        .map(|n| n.to_string())
        .unwrap_or_else(|| folder_name(&canonical));

    let project = projects_repo::insert(&conn, &name, None)?;
    let repository = repositories_repo::insert(&conn, &project.id, &root_path, None, &default_branch)?;
    projects_repo::update_last_opened(&conn, &project.id)?;
    let project = projects_repo::get_by_id(&conn, &project.id)?
        .ok_or_else(|| AppError::NotFound("project disappeared right after being created".to_string()))?;

    Ok(to_dto(project, repository))
}

/// Registers (or re-opens) the git repository at `path` as a project.
/// Errors if `path` doesn't exist or isn't a git repository — never
/// fabricates a project for a folder that isn't actually one.
#[tauri::command]
pub async fn open_project(app: AppHandle, path: String) -> Result<ProjectDto, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        open_or_register(&state, &path, None)
    })
    .await
}

/// Creates `path` (if missing), `git init`s it if it isn't already a
/// repository, then registers it as a project named `name`.
#[tauri::command]
pub async fn init_project(app: AppHandle, path: String, name: String) -> Result<ProjectDto, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let root = PathBuf::from(&path);
        std::fs::create_dir_all(&root)?;

        if !state.git_service.is_git_repository(&root) {
            state.git_service.init(&root)?;
        }

        let name = name.trim();
        let name = if name.is_empty() { None } else { Some(name) };
        open_or_register(&state, &path, name)
    })
    .await
}

/// All known projects, most recently updated first.
#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectDto>, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let projects = projects_repo::list_all(&conn)?;
        let mut dtos = Vec::with_capacity(projects.len());
        for project in projects {
            // A project without a registered repository shouldn't exist in
            // Phase 1 (every project is created alongside one), but skip
            // rather than error if it somehow does.
            if let Some(repository) = repositories_repo::get_by_project_id(&conn, &project.id)? {
                dtos.push(to_dto(project, repository));
            }
        }
        Ok(dtos)
    })
    .await
}

#[tauri::command]
pub fn get_project(_state: State<AppState>, _project_id: String) -> AppResult<Option<Project>> {
    todo!("M2+: db::repository::projects::get_by_id, joined with its repository if the frontend needs one")
}

#[tauri::command]
pub fn create_project(
    _state: State<AppState>,
    _name: String,
    _description: Option<String>,
) -> AppResult<Project> {
    todo!("M2+: a project with no repository yet — not needed until multi-repo projects land")
}

#[tauri::command]
pub fn list_agents(_state: State<AppState>, _project_id: String) -> AppResult<Vec<Agent>> {
    todo!("M5: agents land with the agent-run execution milestone")
}

#[tauri::command]
pub fn list_tasks(_state: State<AppState>, _project_id: String) -> AppResult<Vec<Task>> {
    todo!("M2+: db::repository::tasks::list_for_project — lands with the Tasks route")
}

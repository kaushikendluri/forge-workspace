//! Commands for the M5 worktree/branch lifecycle skeleton: creating agents,
//! listing them, and the "Start" action that creates a real git worktree +
//! branch for a task. No model call or tool loop happens here — that's M6.
//! `Agent`/`Workspace` (from `db::models`) are returned directly as the
//! frontend-facing DTOs; they already mirror the DB rows 1:1 and match
//! `src/types/db.ts`'s `Agent`/`Workspace` types, so no separate wrapper
//! struct is needed the way `ProjectDto` needs one (which joins two tables).

use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::commands::run_blocking;
use crate::db::models::{Agent, Workspace, WorkspaceKind};
use crate::db::repository::{
    agent_runs as agent_runs_repo, agent_skills as agent_skills_repo, agents as agents_repo,
    model_configs as model_configs_repo, repositories as repositories_repo, workspaces as workspaces_repo,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use rusqlite::Connection;

/// Lowercases `name`, collapses runs of non-alphanumeric characters into a
/// single `-`, and trims leading/trailing `-` — for embedding a human agent
/// name inside a git branch name. Falls back to `"agent"` if that leaves
/// nothing (e.g. a name made entirely of emoji/punctuation).
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "agent".to_string()
    } else {
        slug
    }
}

/// Creates a new agent (status `idle`) for `project_id`'s (Phase 1: single)
/// repository. Phase 5 M19: `skill_id`, when given, must name an existing
/// `agent_skills` row — its `instructions` are copied into the new agent's
/// `system_prompt` and its id recorded as `skill_id` (real application, not
/// a stored-but-unused reference — see `agents_repo::insert`'s own docs).
/// `None` is today's plain, unrestricted agent, unchanged.
#[tauri::command]
pub async fn create_agent(app: AppHandle, project_id: String, name: String, skill_id: Option<String>) -> Result<Agent, String> {
    run_blocking(move || -> AppResult<Agent> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;

        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::InvalidInput("agent name cannot be empty".to_string()));
        }

        let repository = repositories_repo::get_by_project_id(&conn, &project_id)?.ok_or_else(|| {
            AppError::NotFound(format!("no repository registered for project {project_id}"))
        })?;

        let skill = match &skill_id {
            Some(id) => {
                Some(agent_skills_repo::get_by_id(&conn, id)?.ok_or_else(|| AppError::NotFound(format!("agent skill {id} not found")))?)
            }
            None => None,
        };

        agents_repo::insert(&conn, &project_id, &repository.id, name, skill.as_ref())
    })
    .await
}

/// Resolves which real `model_id` (the string the Anthropic API itself
/// takes, e.g. `"claude-sonnet-5"`) a new run of `agent` should use. Phase 5
/// M19: if `agent` was created from a skill that names a
/// `preferred_model_id` (and that `model_configs` row still exists), that
/// override wins; otherwise this falls back to exactly today's behavior —
/// the configured default model config, or the hardcoded
/// `"claude-sonnet-5"` if even that is missing.
fn resolve_model_id_for_agent(conn: &Connection, agent: &Agent) -> AppResult<String> {
    if let Some(skill_id) = &agent.skill_id {
        if let Some(skill) = agent_skills_repo::get_by_id(conn, skill_id)? {
            if let Some(preferred_model_id) = &skill.preferred_model_id {
                if let Some(model) = model_configs_repo::get_by_id(conn, preferred_model_id)? {
                    return Ok(model.model_id);
                }
            }
        }
    }
    Ok(model_configs_repo::get_default(conn)?.map(|m| m.model_id).unwrap_or_else(|| "claude-sonnet-5".to_string()))
}

/// All agents for `project_id`, most recently created first.
#[tauri::command]
pub async fn list_agents(app: AppHandle, project_id: String) -> Result<Vec<Agent>, String> {
    run_blocking(move || -> AppResult<Vec<Agent>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agents_repo::list_for_project(&conn, &project_id)
    })
    .await
}

/// The M5-scoped "Start" action: creates a real git worktree + branch for
/// `agent_id`'s repository and records a `queued` agent run — no model call
/// is made here (that's M6). If worktree creation fails (branch name
/// collision, path already exists, ...), git's real error propagates and
/// nothing is written to the database — there's no half-created row to clean
/// up. The agent's own `status` is left `idle`: a workspace now exists, but
/// no execution has actually started, and the schema doesn't (deliberately)
/// have a "workspace ready" agent status worth inventing for that.
#[tauri::command]
pub async fn start_worktree_for_agent(
    app: AppHandle,
    agent_id: String,
    task_prompt: String,
) -> Result<Workspace, String> {
    run_blocking(move || -> AppResult<Workspace> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;

        let task_prompt = task_prompt.trim();
        if task_prompt.is_empty() {
            return Err(AppError::InvalidInput("task description cannot be empty".to_string()));
        }

        let agent = agents_repo::get_by_id(&conn, &agent_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent {agent_id} not found")))?;
        let repository = repositories_repo::get_by_id(&conn, &agent.repository_id)?.ok_or_else(|| {
            AppError::NotFound(format!("repository {} not found", agent.repository_id))
        })?;

        let repo_root = PathBuf::from(&repository.root_path);
        let base_branch = state
            .git_service
            .current_branch(&repo_root)?
            .unwrap_or_else(|| repository.default_branch.clone());

        let run_id = Uuid::new_v4().to_string();
        let short_run_id = &run_id[..8];
        let branch_name = format!("forge/agent/{}/{}", slugify(&agent.name), short_run_id);

        let worktrees_root = repo_root.join(".forge-workspace").join("worktrees");
        std::fs::create_dir_all(&worktrees_root)?;
        let worktree_path = worktrees_root.join(&run_id);

        // Real `git worktree add` — fails honestly (branch collision, path
        // already exists, base branch missing, ...) without writing
        // anything to the database first.
        state
            .git_service
            .add_worktree(&repo_root, &worktree_path, &branch_name, &base_branch)?;

        let model_id = resolve_model_id_for_agent(&conn, &agent)?;

        let run = agent_runs_repo::insert_queued(&conn, &agent.id, task_prompt, &model_id)?;

        let workspace_path = worktree_path.to_string_lossy().to_string();
        let workspace = workspaces_repo::insert(
            &conn,
            &repository.id,
            Some(&run.id),
            WorkspaceKind::Agent,
            &workspace_path,
            &branch_name,
            Some(&base_branch),
            None,
        )?;

        agent_runs_repo::set_workspace_id(&conn, &run.id, &workspace.id)?;

        Ok(workspace)
    })
    .await
}

/// Removes the worktree backing `workspace_id` and marks it `removed`. Fails
/// (rather than force-removing) if the worktree has uncommitted changes —
/// see `GitService::remove_worktree`'s docs.
#[tauri::command]
pub async fn remove_agent_workspace(app: AppHandle, workspace_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;

        let workspace = workspaces_repo::get_by_id(&conn, &workspace_id)?
            .ok_or_else(|| AppError::NotFound(format!("workspace {workspace_id} not found")))?;
        let repository = repositories_repo::get_by_id(&conn, &workspace.repository_id)?.ok_or_else(|| {
            AppError::NotFound(format!("repository {} not found", workspace.repository_id))
        })?;

        let repo_root = PathBuf::from(&repository.root_path);
        let worktree_path = PathBuf::from(&workspace.path);
        state.git_service.remove_worktree(&repo_root, &worktree_path)?;
        workspaces_repo::mark_removed(&conn, &workspace.id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_collapses_punctuation() {
        assert_eq!(slugify("Refactor Bot"), "refactor-bot");
        assert_eq!(slugify("  Weird!! Name__2  "), "weird-name-2");
        assert_eq!(slugify("already-slug"), "already-slug");
    }

    #[test]
    fn slugify_falls_back_when_nothing_alphanumeric() {
        assert_eq!(slugify("!!!"), "agent");
        assert_eq!(slugify(""), "agent");
    }

    fn setup_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::run_migrations(&mut conn).expect("run migrations");
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')", []).expect("insert project");
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", [])
            .expect("insert repository");
        conn
    }

    /// Phase 5 M19's core "skill application" property for model
    /// selection: an agent created from a skill with a real
    /// `preferred_model_id` must have *that* model resolved for its runs,
    /// overriding the configured default — not just store the preference
    /// and ignore it.
    #[test]
    fn resolve_model_id_for_agent_prefers_the_skills_model_over_the_default() {
        let conn = setup_conn();
        // The seeded default is 'default-sonnet' / 'claude-sonnet-5'. Add a
        // second, non-default model config for the skill to prefer.
        conn.execute(
            "INSERT INTO model_configs (id, provider, model_id, display_name, is_default, max_output_tokens) \
             VALUES ('opus-cfg', 'anthropic', 'claude-opus-5', 'Claude Opus 5', 0, 8192)",
            [],
        )
        .expect("insert model config");

        let skill = crate::db::repository::agent_skills::insert(
            &conn,
            "Opus Preferring Skill",
            None,
            "instructions",
            &crate::db::repository::agent_skills::default_tools_json(),
            Some("opus-cfg"),
        )
        .expect("insert skill");

        let agent = agents_repo::insert(&conn, "p1", "r1", "Opus Agent", Some(&skill)).expect("insert agent");
        let model_id = resolve_model_id_for_agent(&conn, &agent).expect("resolve_model_id_for_agent");
        assert_eq!(model_id, "claude-opus-5");
    }

    /// No skill at all must fall back to exactly today's behavior: the
    /// configured default model config.
    #[test]
    fn resolve_model_id_for_agent_falls_back_to_default_without_a_skill() {
        let conn = setup_conn();
        let agent = agents_repo::insert(&conn, "p1", "r1", "Plain Agent", None).expect("insert agent");
        let model_id = resolve_model_id_for_agent(&conn, &agent).expect("resolve_model_id_for_agent");
        assert_eq!(model_id, "claude-sonnet-5");
    }

    /// A skill with no `preferred_model_id` set at all must also fall back
    /// to the default — the override is opt-in per skill.
    #[test]
    fn resolve_model_id_for_agent_falls_back_to_default_when_skill_has_no_preference() {
        let conn = setup_conn();
        let skill = crate::db::repository::agent_skills::insert(
            &conn,
            "No Preference Skill",
            None,
            "instructions",
            &crate::db::repository::agent_skills::default_tools_json(),
            None,
        )
        .expect("insert skill");
        let agent = agents_repo::insert(&conn, "p1", "r1", "Agent", Some(&skill)).expect("insert agent");
        let model_id = resolve_model_id_for_agent(&conn, &agent).expect("resolve_model_id_for_agent");
        assert_eq!(model_id, "claude-sonnet-5");
    }
}

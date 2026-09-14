//! M6 commands: starting/stopping the real agent tool-calling loop
//! (`agent::tool_loop::run_agent_loop`) for an already-`queued` run (created
//! by M5's `commands::agent_commands::start_worktree_for_agent`), and
//! reading back everything `AgentDetail.tsx` renders — the run row itself,
//! its tool calls, its activity feed, and a real diff of what changed in its
//! workspace.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::agent::tool_loop::run_agent_loop;
use crate::commands::run_blocking;
use crate::db::models::{ActivityEvent, AgentRun, AgentRunStatus, ToolCall};
use crate::db::repository::{
    activity_events as activity_events_repo, agent_runs as agent_runs_repo, tool_calls as tool_calls_repo,
    workspaces as workspaces_repo,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// One changed file in a run's workspace, for `AgentDetail.tsx`'s diff view.
/// `src-tauri/src/git/mod.rs`'s `GitFileDiff` carries no path (the Changes
/// tab already knows which file it asked for), so this flattens it together
/// with the path that produced it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunFileDiffDto {
    pub path: String,
    pub original: String,
    pub modified: String,
    pub is_new_file: bool,
    pub is_deleted: bool,
}

/// Starts the tool-calling loop for `agent_run_id`, which must currently be
/// `queued` (i.e. `start_worktree_for_agent` already created its worktree).
/// Registers the run's `CancellationToken` in `AppState.active_runs`
/// *before* spawning the loop, so a `stop_agent_run` call made immediately
/// after this returns is guaranteed to find it.
#[tauri::command]
pub async fn start_agent_run(app: AppHandle, agent_run_id: String) -> Result<(), String> {
    let cancel = CancellationToken::new();
    let app_for_check = app.clone();
    let run_id_for_check = agent_run_id.clone();
    let cancel_for_check = cancel.clone();

    run_blocking(move || -> AppResult<()> {
        let state = app_for_check.state::<AppState>();
        let conn = state.db.get()?;
        let run = agent_runs_repo::get_by_id(&conn, &run_id_for_check)?
            .ok_or_else(|| AppError::NotFound(format!("agent run {run_id_for_check} not found")))?;
        if run.status != AgentRunStatus::Queued {
            return Err(AppError::InvalidInput(format!(
                "agent run {run_id_for_check} is not queued (status: {:?}) — it may already be running or finished",
                run.status
            )));
        }

        let mut active_runs =
            state.active_runs.lock().map_err(|_| AppError::Other("active runs registry lock poisoned".to_string()))?;
        if active_runs.contains_key(&run_id_for_check) {
            return Err(AppError::InvalidInput(format!("agent run {run_id_for_check} is already active")));
        }
        active_runs.insert(run_id_for_check.clone(), cancel_for_check);
        Ok(())
    })
    .await?;

    let app_for_loop = app.clone();
    tauri::async_runtime::spawn(async move {
        run_agent_loop(app_for_loop, agent_run_id, cancel).await;
    });
    Ok(())
}

/// Cancels a currently-active run: fires its `CancellationToken`, which the
/// loop observes at its next check point (before a model call, before each
/// tool call) and which `AnthropicClient::stream_turn`/`run_command` observe
/// immediately via `tokio::select!` — tearing down an in-flight HTTP stream
/// or killing an in-flight child process rather than waiting for either to
/// finish naturally. Errors (rather than silently no-op-ing) if `agent_run_id`
/// isn't currently active, since that's almost always a stale UI state.
#[tauri::command]
pub async fn stop_agent_run(app: AppHandle, agent_run_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let active_runs =
            state.active_runs.lock().map_err(|_| AppError::Other("active runs registry lock poisoned".to_string()))?;
        match active_runs.get(&agent_run_id) {
            Some(token) => {
                token.cancel();
                Ok(())
            }
            None => Err(AppError::NotFound(format!("agent run {agent_run_id} is not currently active"))),
        }
    })
    .await
}

/// The current state of one run.
#[tauri::command]
pub async fn get_agent_run(app: AppHandle, agent_run_id: String) -> Result<AgentRun, String> {
    run_blocking(move || -> AppResult<AgentRun> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_runs_repo::get_by_id(&conn, &agent_run_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))
    })
    .await
}

/// All runs for `agent_id`, most recently started first — `AgentDetail.tsx`
/// uses the first entry as "the current run" for that agent (Phase 2 scope
/// is one agent, one run at a time; a full run-history view is later work).
#[tauri::command]
pub async fn list_agent_runs(app: AppHandle, agent_id: String) -> Result<Vec<AgentRun>, String> {
    run_blocking(move || -> AppResult<Vec<AgentRun>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_runs_repo::list_for_agent(&conn, &agent_id)
    })
    .await
}

/// All tool calls for a run, in call order.
#[tauri::command]
pub async fn list_tool_calls(app: AppHandle, agent_run_id: String) -> Result<Vec<ToolCall>, String> {
    run_blocking(move || -> AppResult<Vec<ToolCall>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        tool_calls_repo::list_for_run(&conn, &agent_run_id)
    })
    .await
}

/// The full activity feed for a run, oldest first.
#[tauri::command]
pub async fn list_activity_events(app: AppHandle, agent_run_id: String) -> Result<Vec<ActivityEvent>, String> {
    run_blocking(move || -> AppResult<Vec<ActivityEvent>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        activity_events_repo::list_for_run(&conn, &agent_run_id)
    })
    .await
}

/// A real diff of everything currently changed in the run's workspace —
/// every staged/unstaged/untracked path from `GitService::status`, each
/// diffed against `HEAD` via `GitService::diff_file` (the same M3 logic the
/// Changes tab uses). This is the *working-tree* diff of the run's own
/// branch, not a merge-base comparison against `base_branch` — for a run
/// that never commits (the common case, since none of the agent's tools
/// auto-commit), those are the same thing; a run that does commit along the
/// way would need a real merge-base diff, which is future work.
#[tauri::command]
pub async fn get_run_diff(app: AppHandle, agent_run_id: String) -> Result<Vec<AgentRunFileDiffDto>, String> {
    run_blocking(move || -> AppResult<Vec<AgentRunFileDiffDto>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;

        let run = agent_runs_repo::get_by_id(&conn, &agent_run_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))?;
        let workspace_id = run
            .workspace_id
            .ok_or_else(|| AppError::InvalidInput(format!("agent run {agent_run_id} has no workspace")))?;
        let workspace = workspaces_repo::get_by_id(&conn, &workspace_id)?
            .ok_or_else(|| AppError::NotFound(format!("workspace {workspace_id} not found")))?;
        let workspace_root = PathBuf::from(&workspace.path);

        let status = state.git_service.status(&workspace_root)?;
        let mut seen: HashSet<String> = HashSet::new();
        seen.extend(status.staged.into_iter().map(|e| e.path));
        seen.extend(status.unstaged.into_iter().map(|e| e.path));
        seen.extend(status.untracked);
        let mut paths: Vec<String> = seen.into_iter().collect();
        paths.sort();

        paths
            .into_iter()
            .map(|path| {
                let diff = state.git_service.diff_file(&workspace_root, &path)?;
                Ok(AgentRunFileDiffDto {
                    path,
                    original: diff.original,
                    modified: diff.modified,
                    is_new_file: diff.is_new_file,
                    is_deleted: diff.is_deleted,
                })
            })
            .collect::<AppResult<Vec<_>>>()
    })
    .await
}

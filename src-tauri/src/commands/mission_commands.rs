//! M8 commands: turning a plain-English objective into a structured,
//! human-approved task plan. `create_mission` runs the one Anthropic call
//! this needs (`orchestrator::planner::propose_plan`) directly on the async
//! command task (unlike M6's `start_agent_run`, there's no multi-step
//! progress to stream — one API call, then the frontend has its answer) and
//! persists the result as real `tasks` rows. `approve_mission_plan` is as
//! far as M8 goes.
//!
//! M9 added `start_mission`/`stop_mission`, which consume `status =
//! 'approved'`: they mirror `commands::agent_run_commands`'s
//! `start_agent_run`/`stop_agent_run` shape exactly — register/look up a
//! `CancellationToken` in `AppState.active_missions`, spawn/cancel the real
//! driver (`orchestrator::scheduler::run_mission`) — with `active_missions`
//! standing in for `active_runs` one level up. M10 changed nothing about
//! this pair of commands themselves — `run_mission` now runs multiple ready
//! tasks concurrently internally, but starting/stopping *the mission* is
//! still exactly one `CancellationToken` registered/cancelled, same as M9.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::commands::run_blocking;
use crate::db::models::{AgentMessage, Mission, MissionStatus, ReviewStatus, Task, TaskPriority};
use crate::db::repository::{
    agent_messages as agent_messages_repo, missions as missions_repo, model_configs as model_configs_repo,
    repositories as repositories_repo, reviews as reviews_repo, tasks as tasks_repo,
};
use crate::error::{AppError, AppResult};
use crate::git::{GitCliService, GitService};
use crate::orchestrator::planner::{self, MissionPlan};
use crate::orchestrator::scheduler::{self, BoardColumn};
use crate::os_adapter;
use crate::secrets;
use crate::state::AppState;

const FALLBACK_MODEL_ID: &str = "claude-sonnet-5";
const FALLBACK_MAX_TOKENS: u32 = 4096;

fn normalize_priority(raw: &str) -> TaskPriority {
    match raw.to_ascii_lowercase().as_str() {
        "low" => TaskPriority::Low,
        "high" => TaskPriority::High,
        _ => TaskPriority::Medium,
    }
}

/// Marks `mission_id` `failed` with the real `message` and returns the
/// updated row — the shared tail of every `create_mission` failure path
/// (no API key, a planner/API error, a malformed plan), so the frontend
/// always gets back a real `Mission` with an honest `errorMessage` to
/// render rather than a bare thrown error.
async fn fail_mission(app: &AppHandle, mission_id: &str, message: &str) -> Result<Mission, String> {
    let app = app.clone();
    let mission_id = mission_id.to_string();
    let message = message.to_string();
    run_blocking(move || -> AppResult<Mission> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        missions_repo::mark_failed(&conn, &mission_id, &message)?;
        missions_repo::get_by_id(&conn, &mission_id)?
            .ok_or_else(|| AppError::NotFound(format!("mission {mission_id} vanished after being marked failed")))
    })
    .await
}

/// Inserts one real `tasks` row per planned task (two passes: every task
/// first, so every plan-local id has a real row and id to resolve against,
/// then `depends_on` edges — a task can depend on one declared later in the
/// same plan), stores the raw plan JSON, and moves the mission to
/// `plan_ready`.
fn persist_plan(conn: &Connection, mission_id: &str, project_id: &str, plan: &MissionPlan) -> AppResult<Mission> {
    let mut temp_to_real: HashMap<String, String> = HashMap::with_capacity(plan.tasks.len());

    for (position, planned) in plan.tasks.iter().enumerate() {
        let priority = normalize_priority(&planned.priority);
        let task = tasks_repo::insert_for_mission(
            conn,
            project_id,
            mission_id,
            &planned.title,
            planned.description.as_deref(),
            &planned.agent_type,
            priority,
            position as i64,
        )?;
        temp_to_real.insert(planned.id.clone(), task.id);
    }

    for planned in &plan.tasks {
        let Some(dep_temp_id) = &planned.depends_on else { continue };
        let Some(real_task_id) = temp_to_real.get(&planned.id) else { continue };
        match temp_to_real.get(dep_temp_id) {
            // A real edge between two distinct tasks in this plan.
            Some(real_dep_id) if real_dep_id != real_task_id => {
                tasks_repo::set_depends_on(conn, real_task_id, Some(real_dep_id))?;
            }
            // Self-dependency (a task naming its own id) — ignore rather
            // than write a self-referential edge.
            Some(_) => {}
            // The model referenced a `depends_on` id that doesn't match any
            // task's own `id` in this plan — ignore that one bad edge
            // rather than failing the whole mission over it.
            None => {}
        }
    }

    let plan_json = serde_json::to_string(plan).map_err(|e| AppError::Other(format!("failed to serialize plan: {e}")))?;
    missions_repo::mark_plan_ready(conn, mission_id, &plan_json)?;
    missions_repo::get_by_id(conn, mission_id)?
        .ok_or_else(|| AppError::NotFound(format!("mission {mission_id} vanished after plan_ready")))
}

/// Creates a mission for `project_id` and runs the planner for `objective`
/// end to end: inserts the `planning` row, makes the one Anthropic call
/// (after confirming an API key is configured — same explicit-failure
/// pattern `agent::tool_loop` uses for M6 runs), and on success persists the
/// plan as real `tasks` rows and moves the mission to `plan_ready`. On any
/// failure (no API key, an API error, a malformed plan) the mission is
/// marked `failed` with the real error and that row is still returned
/// (`Ok`, not `Err`) — the frontend renders the failure from
/// `mission.errorMessage` rather than catching a thrown error, matching how
/// `AgentDetail.tsx` renders a failed run's `errorMessage` today.
#[tauri::command]
pub async fn create_mission(app: AppHandle, project_id: String, objective: String) -> Result<Mission, String> {
    let objective = objective.trim().to_string();
    if objective.is_empty() {
        return Err(AppError::InvalidInput("mission objective cannot be empty".to_string()).to_string());
    }

    let project_id_for_setup = project_id.clone();
    let objective_for_setup = objective.clone();
    let (mission, repo_root, model_id, max_tokens) = run_blocking({
        let app = app.clone();
        move || -> AppResult<(Mission, PathBuf, String, u32)> {
            let state = app.state::<AppState>();
            let conn = state.db.get()?;

            let repository = repositories_repo::get_by_project_id(&conn, &project_id_for_setup)?.ok_or_else(|| {
                AppError::NotFound(format!("no repository registered for project {project_id_for_setup}"))
            })?;

            let mission = missions_repo::insert(&conn, &project_id_for_setup, &objective_for_setup)?;

            let default_model = model_configs_repo::get_default(&conn)?;
            let model_id = default_model.as_ref().map(|m| m.model_id.clone()).unwrap_or_else(|| FALLBACK_MODEL_ID.to_string());
            let max_tokens = default_model.map(|m| m.max_output_tokens as u32).unwrap_or(FALLBACK_MAX_TOKENS);

            Ok((mission, PathBuf::from(repository.root_path), model_id, max_tokens))
        }
    })
    .await?;

    // `keyring` is a synchronous OS call — same pattern `agent::tool_loop`
    // uses before starting an M6 run: never hang or silently produce an
    // empty plan when no key is configured, fail the mission clearly.
    let api_key = tauri::async_runtime::spawn_blocking(|| secrets::get_secret(secrets::ANTHROPIC_API_KEY))
        .await
        .map_err(|e| format!("API key lookup panicked: {e}"))?
        .map_err(|e| e.to_string())?;
    let Some(api_key) = api_key else {
        let msg = "No Anthropic API key is configured. Add one in Settings, then try again.".to_string();
        return fail_mission(&app, &mission.id, &msg).await;
    };

    let client = match crate::agent::anthropic_client::AnthropicClient::new(api_key) {
        Ok(client) => client,
        Err(e) => return fail_mission(&app, &mission.id, &e.to_string()).await,
    };

    let os_adapter = os_adapter::current();
    let git_service: Box<dyn GitService> = Box::new(GitCliService::new(os_adapter.as_ref()));
    // No live progress to cancel mid-flight for a single structured-output
    // call (unlike a multi-turn agent run) — a fresh, never-fired token is
    // enough to satisfy `request_structured_tool_call`'s signature.
    let cancel = CancellationToken::new();

    let plan_result =
        planner::propose_plan(&client, &model_id, max_tokens, &repo_root, git_service.as_ref(), &objective, &cancel).await;

    let plan = match plan_result {
        Ok(plan) => plan,
        Err(e) => return fail_mission(&app, &mission.id, &e.to_string()).await,
    };

    let mission_id = mission.id.clone();
    let project_id_for_persist = project_id.clone();
    run_blocking({
        let app = app.clone();
        move || -> AppResult<Mission> {
            let state = app.state::<AppState>();
            let conn = state.db.get()?;
            persist_plan(&conn, &mission_id, &project_id_for_persist, &plan)
        }
    })
    .await
}

/// Records human approval of a `plan_ready` mission. Nothing yet consumes
/// `approved` to start execution — that's a later milestone.
#[tauri::command]
pub async fn approve_mission_plan(app: AppHandle, mission_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let mission = missions_repo::get_by_id(&conn, &mission_id)?
            .ok_or_else(|| AppError::NotFound(format!("mission {mission_id} not found")))?;
        if mission.status != MissionStatus::PlanReady {
            return Err(AppError::InvalidInput(format!(
                "mission {mission_id} is not awaiting approval (status: {:?})",
                mission.status
            )));
        }
        missions_repo::mark_approved(&conn, &mission_id)
    })
    .await
}

#[tauri::command]
pub async fn get_mission(app: AppHandle, mission_id: String) -> Result<Mission, String> {
    run_blocking(move || -> AppResult<Mission> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        missions_repo::get_by_id(&conn, &mission_id)?
            .ok_or_else(|| AppError::NotFound(format!("mission {mission_id} not found")))
    })
    .await
}

/// All missions for `project_id`, most recently created first.
#[tauri::command]
pub async fn list_missions(app: AppHandle, project_id: String) -> Result<Vec<Mission>, String> {
    run_blocking(move || -> AppResult<Vec<Mission>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        missions_repo::list_for_project(&conn, &project_id)
    })
    .await
}

/// The tasks `mission_id`'s plan proposed, in plan order.
#[tauri::command]
pub async fn list_mission_tasks(app: AppHandle, mission_id: String) -> Result<Vec<Task>, String> {
    run_blocking(move || -> AppResult<Vec<Task>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        tasks_repo::list_for_mission(&conn, &mission_id)
    })
    .await
}

/// M11: one row of `Tasks.tsx`'s Kanban board — a real `Task` (every field
/// `list_mission_tasks` already returns, flattened) plus its derived
/// `column` (`orchestrator::scheduler::compute_board_columns`). Mirrors
/// `src/types/db.ts`'s `TaskBoardEntryDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBoardEntryDto {
    #[serde(flatten)]
    pub task: Task,
    pub column: BoardColumn,
    /// M14: the latest review's score for this task's agent run, if any has
    /// ever completed (`pending`, `passed`, or `failed`) — `None` when no
    /// review was ever requested for it. Derived from the same
    /// `reviews_repo::latest_reviews_for_runs` batch lookup that decides
    /// `column`, not a separate query.
    pub review_score: Option<i64>,
    /// M14: the latest review's status for this task's agent run, mirroring
    /// `review_score` above.
    pub review_status: Option<ReviewStatus>,
}

/// M11: `mission_id`'s tasks (plan order), each labeled with its derived
/// board column — the read side `Tasks.tsx`'s Kanban board renders from.
/// Purely a label on top of the same `tasks` rows `list_mission_tasks`
/// already returns: nothing here writes anything, and `column` can never
/// disagree with what the scheduler would actually do next, since it's
/// computed with the scheduler's own `classify_task` (see
/// `orchestrator::scheduler::board_column_for_task`'s docs). M14: also folds
/// in each task's latest review (`reviews_repo::latest_reviews_for_runs`,
/// one batched query) — a `done` task with a currently-`pending` review
/// renders as the `Review` column instead of jumping straight to `Complete`,
/// and every task carries its latest review's score/status (if any) for a
/// board-level badge.
#[tauri::command]
pub async fn list_mission_board(app: AppHandle, mission_id: String) -> Result<Vec<TaskBoardEntryDto>, String> {
    run_blocking(move || -> AppResult<Vec<TaskBoardEntryDto>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let tasks = tasks_repo::list_for_mission(&conn, &mission_id)?;
        let run_ids: Vec<String> = tasks.iter().filter_map(|t| t.agent_run_id.clone()).collect();
        let latest_reviews = reviews_repo::latest_reviews_for_runs(&conn, &run_ids)?;
        let pending_review_run_ids: HashSet<String> = latest_reviews
            .iter()
            .filter(|(_, review)| review.status == ReviewStatus::Pending)
            .map(|(run_id, _)| run_id.clone())
            .collect();
        let columns = scheduler::compute_board_columns(&tasks, &pending_review_run_ids);
        Ok(tasks
            .into_iter()
            .map(|t| {
                let column = columns.get(&t.id).copied().unwrap_or(BoardColumn::Backlog);
                let review = t.agent_run_id.as_deref().and_then(|id| latest_reviews.get(id));
                TaskBoardEntryDto {
                    review_score: review.map(|r| r.score),
                    review_status: review.map(|r| r.status),
                    task: t,
                    column,
                }
            })
            .collect())
    })
    .await
}

/// M11: the full agent-to-agent message log for `mission_id`, oldest first
/// — the `send_message` tool (`agent::tools`) is the only thing that writes
/// to this table; this is the read side for Mission Control's messages
/// panel.
#[tauri::command]
pub async fn list_agent_messages(app: AppHandle, mission_id: String) -> Result<Vec<AgentMessage>, String> {
    run_blocking(move || -> AppResult<Vec<AgentMessage>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_messages_repo::list_for_mission(&conn, &mission_id)
    })
    .await
}

/// Starts real execution of an `approved` mission's plan — spawns
/// `orchestrator::scheduler::run_mission`, which walks the task dependency
/// graph and runs ready tasks through the real M5/M6 agent pipeline, up to
/// `agent.max_parallel_agents` of them concurrently (M10; M9 ran one at a
/// time). Registers the mission's `CancellationToken` in
/// `AppState.active_missions` *before* spawning the scheduler, mirroring
/// `agent_run_commands::start_agent_run` exactly, so a `stop_mission` call
/// made immediately after this returns is guaranteed to find it. The actual
/// `status = 'running'` transition (and its event) happens inside
/// `run_mission` itself, not here — same reason `start_agent_run` doesn't
/// call `mark_running` either: the driver is what's actually about to do
/// the work.
#[tauri::command]
pub async fn start_mission(app: AppHandle, mission_id: String) -> Result<(), String> {
    let cancel = CancellationToken::new();
    let app_for_check = app.clone();
    let mission_id_for_check = mission_id.clone();
    let cancel_for_check = cancel.clone();

    run_blocking(move || -> AppResult<()> {
        let state = app_for_check.state::<AppState>();
        let conn = state.db.get()?;
        let mission = missions_repo::get_by_id(&conn, &mission_id_for_check)?
            .ok_or_else(|| AppError::NotFound(format!("mission {mission_id_for_check} not found")))?;
        if mission.status != MissionStatus::Approved {
            return Err(AppError::InvalidInput(format!(
                "mission {mission_id_for_check} is not approved (status: {:?}) — it may already be running or finished",
                mission.status
            )));
        }

        let mut active_missions = state
            .active_missions
            .lock()
            .map_err(|_| AppError::Other("active missions registry lock poisoned".to_string()))?;
        if active_missions.contains_key(&mission_id_for_check) {
            return Err(AppError::InvalidInput(format!("mission {mission_id_for_check} is already running")));
        }
        active_missions.insert(mission_id_for_check.clone(), cancel_for_check);
        Ok(())
    })
    .await?;

    let app_for_loop = app.clone();
    tauri::async_runtime::spawn(async move {
        scheduler::run_mission(app_for_loop, mission_id, cancel).await;
    });
    Ok(())
}

/// Cancels a currently-running mission — fires its `CancellationToken`,
/// which `run_mission` observes and forwards as a real `stop_agent_run`
/// call against *every* task run currently active for this mission (M10:
/// there may be several at once, up to `agent.max_parallel_agents` — each
/// one's own poll loop observes this same token independently, see
/// `orchestrator::scheduler`'s module docs). Errors if `mission_id` isn't
/// currently active, mirroring `agent_run_commands::stop_agent_run`.
#[tauri::command]
pub async fn stop_mission(app: AppHandle, mission_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let active_missions = state
            .active_missions
            .lock()
            .map_err(|_| AppError::Other("active missions registry lock poisoned".to_string()))?;
        match active_missions.get(&mission_id) {
            Some(token) => {
                token.cancel();
                Ok(())
            }
            None => Err(AppError::NotFound(format!("mission {mission_id} is not currently active"))),
        }
    })
    .await
}

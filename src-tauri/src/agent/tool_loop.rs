//! The agent run loop itself: `run_agent_loop` drives a `queued` agent run
//! to `completed`/`failed`/`stopped`, calling the Anthropic Messages API and
//! dispatching whatever tools it asks for (via `agent::executor`) inside the
//! run's isolated git worktree, persisting and streaming every step.
//!
//! Started by `commands::agent_run_commands::start_agent_run`, which
//! registers the run's `CancellationToken` in `AppState.active_runs` and
//! spawns this via `tauri::async_runtime::spawn` — so this function itself
//! never returns a `Result` the caller could see; every outcome (including
//! an unexpected internal error) is instead written to the `agent_runs` row
//! and emitted as a `status-changed` event, since nothing is polling this
//! task's return value.

use std::path::PathBuf;
use std::time::Duration;

use rusqlite::Connection;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::db::models::{
    Agent, ActivityEventType, AgentRun, AgentRunStatus, AgentRunStopReason, AgentStatus, NotificationType, Task, Workspace,
};
use crate::db::repository::{
    activity_events as activity_events_repo, agent_runs as agent_runs_repo, agents as agents_repo,
    model_configs as model_configs_repo, notifications as notifications_repo, settings as settings_repo,
    tasks as tasks_repo, workspaces as workspaces_repo,
};
use crate::db::DbConnection;
use crate::error::{AppError, AppResult};
use crate::git::{GitCliService, GitService};
use crate::os_adapter;
use crate::secrets;
use crate::state::AppState;

use super::anthropic_client::{AnthropicClient, AssistantContentBlock, ContentBlockParam, MessageParam, StreamOutcome};
use super::events as agent_events;
use super::executor::{run_one_tool_call, ExecutedTool};
use super::schema::{all_tool_definitions, send_message_tool_definition};
use super::test_fix::{TestFixEvent, TestFixTracker};
use super::tools::{MissionContext, ToolContext};

const DEFAULT_MAX_ITERATIONS: i64 = 40;
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_TOKENS: u32 = 8192;
/// M13: default cap for `agent.max_test_fix_attempts` — how many times the
/// self-healing test-fix cycle (see `agent::test_fix`) may retry
/// `run_tests` after a failure before the run stops itself rather than
/// looping. Deliberately its own, much smaller budget than
/// `DEFAULT_MAX_ITERATIONS`: 3 genuine fix attempts is already a lot for one
/// failing test suite, and a much larger `max_iterations` value shouldn't
/// let this specific pathological pattern (fail, "fix", fail, "fix", ...)
/// run any longer than that.
const DEFAULT_MAX_TEST_FIX_ATTEMPTS: i64 = 3;
/// Three tool calls in a row coming back as errors is treated as the agent
/// being stuck (wrong command, missing dependency, repeatedly malformed
/// input, ...) rather than something more retries will fix.
const MAX_CONSECUTIVE_TOOL_ERRORS: u32 = 3;
/// Delay before the single retry attempt on an Anthropic API error (network
/// blip, transient 5xx/overloaded response, ...). Not exponential — this
/// loop only ever retries once, so a fixed backoff is all there is to tune.
const RETRY_BACKOFF: Duration = Duration::from_secs(2);

/// Removes `run_id`'s entry from `AppState.active_runs` on drop — guarantees
/// cleanup on *every* exit path out of `run_agent_loop` (normal completion,
/// an early `return`, or an unexpected panic unwind) without repeating the
/// removal call at each return site.
struct ActiveRunGuard {
    app: AppHandle,
    run_id: String,
}

impl Drop for ActiveRunGuard {
    fn drop(&mut self) {
        if let Some(state) = self.app.try_state::<AppState>() {
            if let Ok(mut runs) = state.active_runs.lock() {
                runs.remove(&self.run_id);
            }
        }
    }
}

/// Drives `agent_run_id` from `queued` to a terminal status. `cancel` is the
/// same `CancellationToken` `start_agent_run` already registered in
/// `AppState.active_runs` before spawning this — passed in (rather than
/// created here) so there is no window between "the command returned Ok"
/// and "a `stop_agent_run` call would actually find a token to cancel".
pub async fn run_agent_loop(app: AppHandle, agent_run_id: String, cancel: CancellationToken) {
    let _guard = ActiveRunGuard { app: app.clone(), run_id: agent_run_id.clone() };

    if let Err(e) = run_agent_loop_inner(&app, &agent_run_id, &cancel).await {
        // A hard, unrecoverable error that happened outside the per-turn/
        // per-tool-call error handling below (e.g. the DB became
        // unreachable, or the run/agent/workspace rows themselves are
        // missing/inconsistent). Best-effort mark the run failed so it
        // doesn't sit `running` forever; if even that fails there is
        // nothing further this task can do.
        let msg = e.to_string();
        let _ = finish_run(&app, &agent_run_id, AgentRunStatus::Failed, Some(AgentRunStopReason::Error), Some(msg.clone()), Some(&msg));
    }
}

fn get_conn(app: &AppHandle) -> AppResult<DbConnection> {
    app.state::<AppState>().db.get().map_err(Into::into)
}

/// Everything the loop needs, loaded once up front.
struct RunSetup {
    agent_run: AgentRun,
    agent: Agent,
    workspace: Workspace,
    max_iterations: i64,
    tool_timeout: Duration,
    test_command: Option<String>,
    lint_command: Option<String>,
    build_command: Option<String>,
    model_max_tokens: u32,
    /// M13: `agent.max_test_fix_attempts` — see [`DEFAULT_MAX_TEST_FIX_ATTEMPTS`].
    max_test_fix_attempts: i64,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// Loads the run/agent/workspace rows plus the M4 settings the loop and its
/// tools need. Settings are read once, up front, rather than per-tool-call —
/// a mid-run settings change (unlikely, since this is a single desktop app
/// with one user) simply takes effect on the next run.
fn load_run_setup(conn: &Connection, agent_run_id: &str) -> AppResult<RunSetup> {
    let agent_run = agent_runs_repo::get_by_id(conn, agent_run_id)?
        .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))?;
    let agent = agents_repo::get_by_id(conn, &agent_run.agent_id)?
        .ok_or_else(|| AppError::NotFound(format!("agent {} not found", agent_run.agent_id)))?;
    let workspace_id = agent_run.workspace_id.clone().ok_or_else(|| {
        AppError::InvalidInput(format!(
            "agent run {agent_run_id} has no workspace — start_worktree_for_agent must succeed before start_agent_run"
        ))
    })?;
    let workspace = workspaces_repo::get_by_id(conn, &workspace_id)?
        .ok_or_else(|| AppError::NotFound(format!("workspace {workspace_id} not found")))?;

    let max_iterations = settings_repo::get(conn, "agent.max_iterations")?
        .and_then(|s| s.value.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_ITERATIONS);
    let tool_timeout_ms = settings_repo::get(conn, "agent.tool_timeout_ms")?
        .and_then(|s| s.value.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_TOOL_TIMEOUT_MS);
    // M12: these live per-project (`project.<project_id>.<kind>_command`),
    // not as one global setting — see `project_detect::project_setting_key`,
    // which `commands::project_commands::open_or_register` also uses to
    // pre-fill them from real detection, and `commands::testing_commands`
    // uses for the Testing tab's manual runs.
    let test_command =
        non_empty(settings_repo::get(conn, &crate::project_detect::project_setting_key(&agent.project_id, "test_command"))?.map(|s| s.value));
    let lint_command =
        non_empty(settings_repo::get(conn, &crate::project_detect::project_setting_key(&agent.project_id, "lint_command"))?.map(|s| s.value));
    let build_command =
        non_empty(settings_repo::get(conn, &crate::project_detect::project_setting_key(&agent.project_id, "build_command"))?.map(|s| s.value));

    let model_max_tokens = model_configs_repo::list(conn)?
        .into_iter()
        .find(|m| m.model_id == agent_run.model_id)
        .map(|m| m.max_output_tokens as u32)
        .unwrap_or(DEFAULT_MAX_TOKENS);

    let max_test_fix_attempts = settings_repo::get(conn, "agent.max_test_fix_attempts")?
        .and_then(|s| s.value.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_TEST_FIX_ATTEMPTS);

    Ok(RunSetup {
        agent_run,
        agent,
        workspace,
        max_iterations,
        tool_timeout: Duration::from_millis(tool_timeout_ms),
        test_command,
        lint_command,
        build_command,
        model_max_tokens,
        max_test_fix_attempts,
    })
}

fn build_system_prompt(setup: &RunSetup) -> String {
    format!(
        "You are an autonomous coding agent named \"{agent_name}\", working inside a dedicated, isolated git \
         worktree at `{workspace_root}` on branch `{branch}` (based on `{base}`). Nothing outside this worktree is \
         reachable through your tools.\n\n\
         Your task:\n{task}\n\n\
         Rules:\n\
         - Use the provided tools to explore, read, and edit the codebase. Read a file before editing it — never \
           assume its contents.\n\
         - All filesystem tools are sandboxed to this worktree; every path must be relative to its root.\n\
         - Prefer the configured `run_tests`/`run_linter`/`run_build` tools over guessing whether a change works — \
           if one isn't configured for this project, it will tell you rather than silently doing nothing.\n\
         - When the task is fully done (or you determine it cannot be completed), call `report_completion` exactly \
           once with a clear summary and whether you succeeded. That is the only way this run ends — stopping \
           without calling it leaves the task incomplete.\n",
        agent_name = setup.agent.name,
        workspace_root = setup.workspace.path,
        branch = setup.workspace.branch_name,
        base = setup.workspace.base_branch.as_deref().unwrap_or("(unknown)"),
        task = setup.agent_run.task_prompt,
    )
}

/// Builds a `test_fix_cycle` activity event's `payload_json` for one
/// `TestFixEvent` — `AgentDetail.tsx`'s activity stream renders `phase` (and
/// `attempt`/`passed`) into "Attempt N: tests failed → investigating" /
/// "Attempt N: retest passed/still failing" text. `test_run_id`, when
/// present, is the real `test_runs` row (M12) this specific `run_tests` call
/// persisted, so the UI can link straight to its full output instead of
/// this event duplicating it.
fn test_fix_event_payload(event: &TestFixEvent, test_run_id: Option<&str>) -> String {
    match event {
        TestFixEvent::Failed { attempt } => {
            serde_json::json!({ "phase": "failed", "attempt": attempt, "testRunId": test_run_id }).to_string()
        }
        TestFixEvent::Retested { attempt, passed } => {
            serde_json::json!({ "phase": "retested", "attempt": attempt, "passed": passed, "testRunId": test_run_id }).to_string()
        }
    }
}

fn to_content_block_param(block: &AssistantContentBlock) -> ContentBlockParam {
    match block {
        AssistantContentBlock::Text(text) => ContentBlockParam::Text { text: text.clone() },
        AssistantContentBlock::ToolUse { id, name, input } => {
            ContentBlockParam::ToolUse { id: id.clone(), name: name.clone(), input: input.clone() }
        }
    }
}

/// One `stream_turn` call, with a single retry-with-backoff on a genuine API
/// error (network failure, non-2xx response, a server-sent `error` event, a
/// malformed stream). A `StreamOutcome::Cancelled` is not retried — it's not
/// a failure, it's `stop_agent_run` having fired.
#[allow(clippy::too_many_arguments)]
async fn call_with_retry(
    app: &AppHandle,
    agent_run_id: &str,
    client: &AnthropicClient,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[MessageParam],
    tools: &[super::anthropic_client::ToolDefinition],
    cancel: &CancellationToken,
) -> AppResult<StreamOutcome> {
    let mut last_err: Option<AppError> = None;
    for attempt in 0..2u8 {
        if attempt > 0 {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(StreamOutcome::Cancelled),
                _ = tokio::time::sleep(RETRY_BACKOFF) => {}
            }
        }

        let app_for_delta = app.clone();
        let run_id_for_delta = agent_run_id.to_string();
        let result = client
            .stream_turn(model, max_tokens, system, messages, tools, cancel, |text: &str| {
                let _ = agent_events::message_delta(&app_for_delta, &run_id_for_delta, text);
            })
            .await;

        match result {
            Ok(outcome) => return Ok(outcome),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::Other("Anthropic API request failed".to_string())))
}

/// Derives the `(NotificationType, title, body)` for a terminal run outcome.
/// `detail` is the specific, human-readable reason for this particular
/// transition (a `report_completion` summary, an API error's text, "stopped
/// by user request", ...) — when the caller has one, it's used verbatim as
/// the body; only a genuinely detail-less transition (which no current call
/// site actually hits) falls back to a `stop_reason`-derived sentence, so
/// this never surfaces a bare "something happened".
fn notification_for_outcome(
    agent_name: &str,
    status: AgentRunStatus,
    stop_reason: Option<AgentRunStopReason>,
    detail: Option<&str>,
) -> (NotificationType, String, Option<String>) {
    let (notification_type, verb) = match status {
        AgentRunStatus::Completed => (NotificationType::AgentCompleted, "completed"),
        AgentRunStatus::Failed => (NotificationType::AgentFailed, "failed"),
        AgentRunStatus::Stopped | AgentRunStatus::Queued | AgentRunStatus::Running => {
            (NotificationType::AgentStopped, "stopped")
        }
    };
    let title = format!("\"{agent_name}\" {verb}");
    let body = detail
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            stop_reason.map(|r| match r {
                AgentRunStopReason::Completed => "The run finished.".to_string(),
                AgentRunStopReason::MaxIterations => "Reached the maximum number of iterations.".to_string(),
                AgentRunStopReason::UserStopped => "Stopped by user request.".to_string(),
                AgentRunStopReason::Error => "The run failed.".to_string(),
                AgentRunStopReason::TestFixBudgetExhausted => {
                    "Reached the maximum number of test-fix attempts.".to_string()
                }
            })
        });
    (notification_type, title, body)
}

/// Writes the run's terminal status, mirrors it onto the owning `agent` row,
/// records a closing activity event, inserts a real notification, and emits
/// the status-changed and notification-created events. Called exactly once
/// per run, from whichever branch of the loop below determines the run is
/// over. `notification_detail` is the specific reason for this transition
/// (see `notification_for_outcome`) — distinct from `error_message` because
/// a successful `report_completion` has a summary worth notifying on even
/// though it isn't an error.
fn finish_run(
    app: &AppHandle,
    agent_run_id: &str,
    status: AgentRunStatus,
    stop_reason: Option<AgentRunStopReason>,
    error_message: Option<String>,
    notification_detail: Option<&str>,
) -> AppResult<()> {
    let agent_id = {
        let conn = get_conn(app)?;
        agent_runs_repo::mark_finished(&conn, agent_run_id, status, stop_reason, error_message.as_deref())?;
        let run = agent_runs_repo::get_by_id(&conn, agent_run_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))?;
        let agent = agents_repo::get_by_id(&conn, &run.agent_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent {} not found", run.agent_id)))?;

        let agent_status = match status {
            AgentRunStatus::Completed => AgentStatus::Completed,
            AgentRunStatus::Failed => AgentStatus::Failed,
            AgentRunStatus::Stopped => AgentStatus::Stopped,
            AgentRunStatus::Queued | AgentRunStatus::Running => AgentStatus::Idle,
        };
        agents_repo::set_status(&conn, &run.agent_id, agent_status)?;

        let event_type = match status {
            AgentRunStatus::Completed => ActivityEventType::RunCompleted,
            AgentRunStatus::Stopped => ActivityEventType::RunStopped,
            _ => ActivityEventType::Error,
        };
        let payload = serde_json::json!({
            "status": status,
            "stopReason": stop_reason,
            "errorMessage": error_message,
        })
        .to_string();
        let event = activity_events_repo::insert(&conn, agent_run_id, None, event_type, &payload)?;
        agent_events::activity(app, agent_run_id, event)?;

        let (notification_type, title, body) = notification_for_outcome(&agent.name, status, stop_reason, notification_detail);
        let notification = notifications_repo::insert(
            &conn,
            Some(&agent.project_id),
            Some(agent_run_id),
            notification_type,
            &title,
            body.as_deref(),
        )?;
        agent_events::notification_created(app, notification)?;

        run.agent_id
    };

    let agent_status = match status {
        AgentRunStatus::Completed => AgentStatus::Completed,
        AgentRunStatus::Failed => AgentStatus::Failed,
        AgentRunStatus::Stopped => AgentStatus::Stopped,
        AgentRunStatus::Queued | AgentRunStatus::Running => AgentStatus::Idle,
    };
    agent_events::run_status_changed(app, agent_run_id, &agent_id, status)?;
    agent_events::agent_status_changed(app, &agent_id, agent_status)?;
    Ok(())
}

async fn run_agent_loop_inner(app: &AppHandle, agent_run_id: &str, cancel: &CancellationToken) -> AppResult<()> {
    let setup = {
        let conn = get_conn(app)?;
        load_run_setup(&conn, agent_run_id)?
    };

    if setup.agent_run.status != AgentRunStatus::Queued {
        // `start_agent_run` already checks this, but the loop re-checks in
        // case it was somehow invoked twice for the same run.
        return Err(AppError::InvalidInput(format!(
            "agent run {agent_run_id} is not queued (status: {:?})",
            setup.agent_run.status
        )));
    }

    // `keyring` is a synchronous OS call; run it on the blocking pool rather
    // than stalling this async task's worker thread.
    let api_key = tauri::async_runtime::spawn_blocking(|| secrets::get_secret(secrets::ANTHROPIC_API_KEY))
        .await
        .map_err(|e| AppError::Other(format!("API key lookup panicked: {e}")))??;
    let Some(api_key) = api_key else {
        let msg = "No Anthropic API key is configured. Add one in Settings, then start this run again.".to_string();
        finish_run(app, agent_run_id, AgentRunStatus::Failed, Some(AgentRunStopReason::Error), Some(msg.clone()), Some(&msg))?;
        return Ok(());
    };

    {
        let conn = get_conn(app)?;
        agent_runs_repo::mark_running(&conn, agent_run_id)?;
        agents_repo::set_status(&conn, &setup.agent.id, AgentStatus::Running)?;
        let payload = serde_json::json!({ "taskPrompt": setup.agent_run.task_prompt }).to_string();
        let event = activity_events_repo::insert(&conn, agent_run_id, None, ActivityEventType::RunStarted, &payload)?;
        agent_events::activity(app, agent_run_id, event)?;
    }
    agent_events::run_status_changed(app, agent_run_id, &setup.agent.id, AgentRunStatus::Running)?;
    agent_events::agent_status_changed(app, &setup.agent.id, AgentStatus::Running)?;

    // M11: resolve, once, whether this run is executing as part of a mission
    // (the scheduler linked a task to it via `tasks_repo::set_agent_run_id`
    // before starting it) — `None` for a solo run started directly from the
    // Agents page. Drives both which tools the model is offered
    // (`send_message` only for a mission-context run) and what
    // `ToolContext::mission_context` carries for the tool's own dispatch.
    let mission_context: Option<MissionContext> = {
        let conn = get_conn(app)?;
        tasks_repo::get_by_agent_run_id(&conn, agent_run_id)?.and_then(|t| {
            let Task { id: task_id, mission_id, .. } = t;
            mission_id.map(|mission_id| MissionContext { mission_id, task_id })
        })
    };
    let db_pool = app.state::<AppState>().db.clone();

    let client = AnthropicClient::new(api_key)?;
    let mut tool_defs = all_tool_definitions();
    if mission_context.is_some() {
        tool_defs.push(send_message_tool_definition());
    }
    let workspace_root = PathBuf::from(&setup.workspace.path);
    let system_prompt = build_system_prompt(&setup);
    let model_id = setup.agent_run.model_id.clone();

    // Fresh instances rather than reaching into `AppState` — both are cheap
    // (a zero-sized adapter struct, a `git` path lookup) and this sidesteps
    // holding a borrow of `AppState` across the `.await` points below.
    let os_adapter = os_adapter::current();
    let git_service: Box<dyn GitService> = Box::new(GitCliService::new(os_adapter.as_ref()));

    let mut messages = vec![MessageParam::user_text(setup.agent_run.task_prompt.clone())];
    let mut consecutive_tool_errors: u32 = 0;
    let mut sequence_number: i64 = 0;
    // M13: the self-healing test-fix cycle's own, bounded tracking — see
    // `agent::test_fix` for the heuristic and the cap's exact semantics.
    let mut test_fix_tracker = TestFixTracker::new();

    for iteration in 1..=setup.max_iterations {
        if cancel.is_cancelled() {
            finish_run(
                app,
                agent_run_id,
                AgentRunStatus::Stopped,
                Some(AgentRunStopReason::UserStopped),
                None,
                Some("Stopped by user request."),
            )?;
            return Ok(());
        }

        let outcome = call_with_retry(
            app,
            agent_run_id,
            &client,
            &model_id,
            setup.model_max_tokens,
            &system_prompt,
            &messages,
            &tool_defs,
            cancel,
        )
        .await;

        let turn = match outcome {
            Ok(StreamOutcome::Turn(turn)) => turn,
            Ok(StreamOutcome::Cancelled) => {
                finish_run(
                    app,
                    agent_run_id,
                    AgentRunStatus::Stopped,
                    Some(AgentRunStopReason::UserStopped),
                    None,
                    Some("Stopped by user request."),
                )?;
                return Ok(());
            }
            Err(e) => {
                let msg = e.to_string();
                finish_run(app, agent_run_id, AgentRunStatus::Failed, Some(AgentRunStopReason::Error), Some(msg.clone()), Some(&msg))?;
                return Ok(());
            }
        };

        {
            let conn = get_conn(app)?;
            agent_runs_repo::record_iteration_usage(&conn, agent_run_id, turn.usage.input_tokens, turn.usage.output_tokens)?;
            let payload = serde_json::json!({
                "iteration": iteration,
                "text": turn.text(),
                "stopReason": turn.stop_reason,
                "usage": { "inputTokens": turn.usage.input_tokens, "outputTokens": turn.usage.output_tokens },
            })
            .to_string();
            let event = activity_events_repo::insert(&conn, agent_run_id, None, ActivityEventType::ModelMessage, &payload)?;
            agent_events::activity(app, agent_run_id, event)?;
        }

        let tool_uses: Vec<(String, String, serde_json::Value)> =
            turn.tool_uses().map(|(id, name, input)| (id.to_string(), name.to_string(), input.clone())).collect();

        if tool_uses.is_empty() {
            // The model stopped talking without calling any tool at all —
            // including `report_completion`. Treated as a (weak) completion
            // rather than a failure: there's no tool error to report, and
            // refusing to end the run would just spin until max_iterations.
            let text = turn.text();
            let detail = if text.trim().is_empty() { None } else { Some(text.as_str()) };
            finish_run(app, agent_run_id, AgentRunStatus::Completed, Some(AgentRunStopReason::Completed), None, detail)?;
            return Ok(());
        }

        let assistant_blocks: Vec<ContentBlockParam> = turn.content.iter().map(to_content_block_param).collect();
        messages.push(MessageParam::assistant(assistant_blocks));

        let ctx = ToolContext {
            workspace_root: workspace_root.clone(),
            git_service: git_service.as_ref(),
            os_adapter: os_adapter.as_ref(),
            test_command: setup.test_command.clone(),
            lint_command: setup.lint_command.clone(),
            build_command: setup.build_command.clone(),
            tool_timeout: setup.tool_timeout,
            cancel: cancel.clone(),
            agent_run_id: agent_run_id.to_string(),
            mission_context: mission_context.clone(),
            db_pool: db_pool.clone(),
            project_id: setup.agent.project_id.clone(),
        };

        let mut tool_results: Vec<ContentBlockParam> = Vec::with_capacity(tool_uses.len());
        let mut ended: Option<(bool, String)> = None;

        for (tool_use_id, tool_name, input) in tool_uses {
            if cancel.is_cancelled() {
                finish_run(
                    app,
                    agent_run_id,
                    AgentRunStatus::Stopped,
                    Some(AgentRunStopReason::UserStopped),
                    None,
                    Some("Stopped by user request."),
                )?;
                return Ok(());
            }

            sequence_number += 1;
            let executed =
                run_one_tool_call(app, &ctx, agent_run_id, sequence_number, &tool_use_id, &tool_name, &input).await?;

            match executed {
                ExecutedTool::ToolResult { block, is_error, test_run_id } => {
                    consecutive_tool_errors = if is_error { consecutive_tool_errors + 1 } else { 0 };
                    tool_results.push(block);

                    // M13: only `run_tests` calls feed the self-healing
                    // test-fix tracker — everything else about "what
                    // happened in between" (write_file/edit_file/other
                    // tool calls) is irrelevant to the heuristic by
                    // construction (see `agent::test_fix` module docs).
                    if tool_name == "run_tests" {
                        let fix_events = test_fix_tracker.observe(is_error);
                        if !fix_events.is_empty() {
                            let conn = get_conn(app)?;
                            for fix_event in &fix_events {
                                let payload = test_fix_event_payload(fix_event, test_run_id.as_deref());
                                let event = activity_events_repo::insert(
                                    &conn,
                                    agent_run_id,
                                    None,
                                    ActivityEventType::TestFixCycle,
                                    &payload,
                                )?;
                                agent_events::activity(app, agent_run_id, event)?;
                            }
                            agent_runs_repo::set_test_fix_attempts(&conn, agent_run_id, test_fix_tracker.attempts())?;
                        }

                        if let Some(attempts) = test_fix_tracker.check_budget(setup.max_test_fix_attempts) {
                            // Flag it clearly in the activity stream, then
                            // stop the run — deliberately `Stopped`, not
                            // `Failed`: the model may well have been doing
                            // legitimate work, this is just a deliberate
                            // "a human should look at this" halt rather than
                            // letting fail→fix→retest run unbounded. This is
                            // also what actually *terminates* the run here —
                            // not `max_iterations`, which could be set far
                            // higher than this budget.
                            let conn = get_conn(app)?;
                            let payload = serde_json::json!({
                                "phase": "budget_exhausted",
                                "attempts": attempts,
                                "cap": setup.max_test_fix_attempts,
                            })
                            .to_string();
                            let event =
                                activity_events_repo::insert(&conn, agent_run_id, None, ActivityEventType::TestFixCycle, &payload)?;
                            agent_events::activity(app, agent_run_id, event)?;
                            drop(conn);

                            let msg = format!(
                                "Test-fix budget exhausted: {attempts} fix attempts (cap {}), tests still failing — \
                                 stopping so a human can look rather than looping indefinitely.",
                                setup.max_test_fix_attempts
                            );
                            finish_run(
                                app,
                                agent_run_id,
                                AgentRunStatus::Stopped,
                                Some(AgentRunStopReason::TestFixBudgetExhausted),
                                None,
                                Some(&msg),
                            )?;
                            return Ok(());
                        }
                    }

                    if consecutive_tool_errors >= MAX_CONSECUTIVE_TOOL_ERRORS {
                        let msg = format!(
                            "{MAX_CONSECUTIVE_TOOL_ERRORS} consecutive tool calls failed — stopping rather than looping."
                        );
                        finish_run(
                            app,
                            agent_run_id,
                            AgentRunStatus::Failed,
                            Some(AgentRunStopReason::Error),
                            Some(msg.clone()),
                            Some(&msg),
                        )?;
                        return Ok(());
                    }
                }
                ExecutedTool::Completion { summary, success } => {
                    ended = Some((success, summary));
                    break;
                }
            }
        }

        if let Some((success, summary)) = ended {
            let status = if success { AgentRunStatus::Completed } else { AgentRunStatus::Failed };
            let stop_reason = Some(if success { AgentRunStopReason::Completed } else { AgentRunStopReason::Error });
            let error_message = if success { None } else { Some(summary.clone()) };
            finish_run(app, agent_run_id, status, stop_reason, error_message, Some(&summary))?;
            return Ok(());
        }

        messages.push(MessageParam::user_tool_results(tool_results));
    }

    let msg = format!("Reached the maximum of {} iterations without calling report_completion.", setup.max_iterations);
    finish_run(app, agent_run_id, AgentRunStatus::Stopped, Some(AgentRunStopReason::MaxIterations), None, Some(&msg))?;
    Ok(())
}

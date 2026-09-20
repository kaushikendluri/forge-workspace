//! M9: walks an `approved` mission's task dependency graph and actually runs
//! each task, **sequentially**, through the existing M5/M6 agent pipeline
//! (`commands::agent_commands::{create_agent,start_worktree_for_agent}` +
//! `commands::agent_run_commands::{start_agent_run,stop_agent_run}`) — the
//! first milestone where a mission produces real running agents and real
//! code changes. True parallel execution of independently-ready tasks is
//! explicitly deferred to M10; this module only ever has one task
//! `in_progress` at a time.
//!
//! Split into two halves:
//!   * Pure graph logic (`evaluate`, `classify_task`, `task_outcome_for_run`)
//!     — no I/O, fully unit-testable, see the tests below.
//!   * The real async driver (`run_mission`), which is what
//!     `commands::mission_commands::start_mission` spawns.
//!
//! ## Status semantics (read this before touching the driver)
//!
//! A task's `depends_on_task_id` (if any) is the *only* thing that gates it.
//! For a `backlog` task with a dependency:
//!   * dependency `done` -> the task is **ready**.
//!   * dependency still `backlog`/`todo`/`in_progress` -> **waiting** (do
//!     nothing this pass).
//!   * dependency `failed`/`blocked`/`cancelled` -> the task can never
//!     succeed as planned, so it's promoted to **blocked** (a task is never
//!     silently attempted anyway once its dependency has failed).
//!   * the task is part of a dependency cycle (shouldn't happen given M8's
//!     plan structure, but detected defensively) -> also **blocked**, with a
//!     distinct reason, rather than looping forever re-evaluating it.
//!
//! `run_mission` awaits one task's whole agent run before starting the next
//! (see `poll_until_terminal`), so at the top of every loop iteration
//! exactly zero tasks are `in_progress` — "nothing is ready and nothing is
//! running" is therefore just "no `backlog` task classifies as ready".
//!
//! When the run backing a task ends:
//!   * `Completed` -> task `done`.
//!   * `Failed`, or `Stopped` for a reason *other* than this mission's own
//!     `stop_mission` (max-iterations, or someone stopping that individual
//!     run directly) -> task `failed`.
//!   * `Stopped` *because* `stop_mission` fired -> task `cancelled`, not
//!     `failed` — nothing about the task itself went wrong.
//!
//! When `stop_mission` fires: the in-flight task (if any) is cancelled via
//! the real `stop_agent_run` (see above) and finishes as `cancelled`; every
//! other task still sitting in `backlog` is also marked `cancelled` (not
//! silently left in `backlog`, which would look indistinguishable from a
//! mission that was simply never started) and the mission itself becomes
//! `stopped` — never `failed`, since a user-requested stop isn't an error.
//!
//! When the mission runs to its natural end (nothing left ready, nothing
//! running): `completed` only if *every* task reached `done`; otherwise
//! `failed`, with `error_message` spelling out exactly which tasks
//! succeeded/failed/were blocked rather than collapsing a partial success
//! into a bare "failed".

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::commands::{agent_commands, agent_run_commands};
use crate::db::models::{AgentRunStatus, MissionStatus, Task, TaskStatus};
use crate::db::repository::{agent_runs as agent_runs_repo, missions as missions_repo, tasks as tasks_repo};
use crate::db::DbConnection;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

use super::events;

/// How often `poll_until_terminal` re-checks an in-flight run's status.
/// M6's `tool_loop` has no completion channel/oneshot to hook into today, so
/// this is a short DB-status poll rather than a push signal — see the
/// module docs' "signal mechanism" note in the M9 handoff report for why
/// that tradeoff was made instead of adding one to `tool_loop.rs`.
const POLL_INTERVAL: Duration = Duration::from_millis(400);

// ---------------------------------------------------------------------
// Pure graph logic
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskReadiness {
    Ready,
    Waiting,
    Blocked { reason: String },
}

/// Follows `start_id`'s `depends_on_task_id` chain upward; `true` if it ever
/// revisits a node (including `start_id` itself, e.g. a task naming itself
/// as its own dependency). Every node has at most one outgoing edge, so this
/// always terminates: either it reaches a task with no dependency (`false`)
/// or it revisits a node within at most `by_id.len() + 1` steps (`true`) —
/// never an unbounded loop.
fn is_in_dependency_cycle(start_id: &str, by_id: &HashMap<&str, &Task>) -> bool {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut current = start_id;
    loop {
        if !visited.insert(current) {
            return true;
        }
        match by_id.get(current) {
            None => return false,
            Some(task) => match task.depends_on_task_id.as_deref() {
                None => return false,
                Some(dep) => current = dep,
            },
        }
    }
}

/// Classifies one `backlog` task's readiness against the current snapshot of
/// its mission's other tasks. See the module docs for the exact semantics.
fn classify_task(task: &Task, by_id: &HashMap<&str, &Task>) -> TaskReadiness {
    if is_in_dependency_cycle(&task.id, by_id) {
        return TaskReadiness::Blocked { reason: "part of a dependency cycle".to_string() };
    }

    let Some(dep_id) = task.depends_on_task_id.as_deref() else {
        return TaskReadiness::Ready;
    };

    match by_id.get(dep_id) {
        None => TaskReadiness::Blocked { reason: format!("depends on task {dep_id}, which no longer exists") },
        Some(dep) => match dep.status {
            TaskStatus::Done => TaskReadiness::Ready,
            TaskStatus::Failed | TaskStatus::Blocked | TaskStatus::Cancelled => TaskReadiness::Blocked {
                reason: format!("its dependency \"{}\" did not complete successfully ({:?})", dep.title, dep.status),
            },
            TaskStatus::Backlog | TaskStatus::Todo | TaskStatus::InProgress => TaskReadiness::Waiting,
        },
    }
}

/// The result of evaluating every currently-`backlog` task in one mission's
/// task set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchedulerEvaluation {
    /// `backlog` task ids ready to start right now, ordered by `position`
    /// (ascending — `run_mission` always starts `ready[0]`, but every ready
    /// id is returned, not just the first, since this milestone still needs
    /// to know *all* currently-ready tasks even though it only starts one).
    pub ready: Vec<String>,
    /// `backlog` task ids that should be promoted to `blocked` this pass,
    /// paired with a human-readable reason.
    pub newly_blocked: Vec<(String, String)>,
}

/// Evaluates every `backlog` task in `tasks` (a snapshot of one mission's
/// full task list) and reports which are ready to run and which should be
/// newly marked `blocked`. Pure — no I/O, no mutation; callers apply the
/// resulting transitions themselves.
pub fn evaluate(tasks: &[Task]) -> SchedulerEvaluation {
    let by_id: HashMap<&str, &Task> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();

    let mut backlog: Vec<&Task> = tasks.iter().filter(|t| t.status == TaskStatus::Backlog).collect();
    backlog.sort_by_key(|t| t.position);

    let mut result = SchedulerEvaluation::default();
    for task in backlog {
        match classify_task(task, &by_id) {
            TaskReadiness::Ready => result.ready.push(task.id.clone()),
            TaskReadiness::Blocked { reason } => result.newly_blocked.push((task.id.clone(), reason)),
            TaskReadiness::Waiting => {}
        }
    }
    result
}

/// What a task's own status should become once its agent run reaches a
/// terminal `AgentRunStatus`. Pure — see the module docs for the exact
/// mapping and why `Stopped` splits into `Failed`/`Cancelled` depending on
/// whether the mission itself was told to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskOutcome {
    Done,
    Failed,
    Cancelled,
}

/// `mission_stop_requested` should be `cancel.is_cancelled()` for the
/// mission-level token at the moment the run finished — `true` means
/// `stop_mission` was the reason this run ended, not a coincidental
/// individual stop/failure. Panics if `status` isn't terminal — callers
/// only reach this after `poll_until_terminal` confirms that.
pub fn task_outcome_for_run(status: AgentRunStatus, mission_stop_requested: bool) -> TaskOutcome {
    match status {
        AgentRunStatus::Completed => TaskOutcome::Done,
        AgentRunStatus::Failed => TaskOutcome::Failed,
        AgentRunStatus::Stopped => {
            if mission_stop_requested {
                TaskOutcome::Cancelled
            } else {
                TaskOutcome::Failed
            }
        }
        AgentRunStatus::Queued | AgentRunStatus::Running => {
            unreachable!("task_outcome_for_run is only called once the run has reached a terminal status")
        }
    }
}

fn is_terminal_run_status(status: AgentRunStatus) -> bool {
    matches!(status, AgentRunStatus::Completed | AgentRunStatus::Failed | AgentRunStatus::Stopped)
}

// ---------------------------------------------------------------------
// The real async driver
// ---------------------------------------------------------------------

fn get_conn(app: &AppHandle) -> AppResult<DbConnection> {
    app.state::<AppState>().db.get().map_err(Into::into)
}

/// Removes `mission_id`'s entry from `AppState.active_missions` on drop —
/// the same "guaranteed cleanup on every exit path" shape as
/// `agent::tool_loop::ActiveRunGuard`.
struct ActiveMissionGuard {
    app: AppHandle,
    mission_id: String,
}

impl Drop for ActiveMissionGuard {
    fn drop(&mut self) {
        if let Some(state) = self.app.try_state::<AppState>() {
            if let Ok(mut missions) = state.active_missions.lock() {
                missions.remove(&self.mission_id);
            }
        }
    }
}

/// Builds the task prompt handed to `start_worktree_for_agent`: the task's
/// own title/description, with a one-line preamble naming the plan's
/// suggested `agent_type` as a light framing rather than a full specialist
/// persona system (out of scope for this milestone) — `tool_loop`'s own
/// system prompt wraps this unchanged, so nothing about the M6 loop itself
/// is touched.
fn build_task_prompt(task: &Task) -> String {
    let mut sections = Vec::new();
    if let Some(agent_type) = task.agent_type.as_deref().filter(|s| !s.trim().is_empty()) {
        sections.push(format!("You are acting as a {agent_type} specialist for this task."));
    }
    sections.push(format!("Task: {}", task.title));
    if let Some(description) = task.description.as_deref().filter(|s| !s.trim().is_empty()) {
        sections.push(description.to_string());
    }
    sections.join("\n\n")
}

/// Creates a real agent for `task` (named after it), starts a real worktree
/// + queued run for it, links the run back onto the task row, then starts
/// the real M6 tool loop for it — calling the exact same
/// `commands::agent_commands`/`commands::agent_run_commands` functions the
/// Agents/AgentDetail UI calls, not a reimplementation of their bodies.
/// Returns the new run's id.
async fn start_task_execution(app: &AppHandle, task: &Task) -> AppResult<String> {
    let agent_name = match task.agent_type.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(agent_type) => format!("{agent_type}: {}", task.title),
        None => task.title.clone(),
    };

    let agent = agent_commands::create_agent(app.clone(), task.project_id.clone(), agent_name).await.map_err(AppError::Other)?;

    let task_prompt = build_task_prompt(task);
    let workspace =
        agent_commands::start_worktree_for_agent(app.clone(), agent.id.clone(), task_prompt).await.map_err(AppError::Other)?;
    let agent_run_id = workspace
        .agent_run_id
        .clone()
        .ok_or_else(|| AppError::Other(format!("workspace {} was created without an agent run id", workspace.id)))?;

    {
        let conn = get_conn(app)?;
        tasks_repo::set_agent_run_id(&conn, &task.id, &agent_run_id)?;
    }

    agent_run_commands::start_agent_run(app.clone(), agent_run_id.clone()).await.map_err(AppError::Other)?;

    Ok(agent_run_id)
}

/// Awaits `agent_run_id` reaching a terminal status, polling
/// `agent_runs` every [`POLL_INTERVAL`]. If `mission_cancel` fires while
/// this is in flight, forwards it as a real `stop_agent_run` call (M6's own
/// cancellation path) exactly once, then keeps polling until the run
/// actually finishes (which `stop_agent_run` only *requests* — the loop
/// still needs to tear down its in-flight HTTP/tool call before the row
/// reaches `stopped`).
async fn poll_until_terminal(app: &AppHandle, agent_run_id: &str, mission_cancel: &CancellationToken) -> AppResult<AgentRunStatus> {
    let mut stop_sent = false;
    loop {
        let run = {
            let conn = get_conn(app)?;
            agent_runs_repo::get_by_id(&conn, agent_run_id)?
                .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))?
        };
        if is_terminal_run_status(run.status) {
            return Ok(run.status);
        }

        if mission_cancel.is_cancelled() && !stop_sent {
            // Best-effort: if the run has raced to a terminal status between
            // our snapshot above and this call, `stop_agent_run` errors
            // (not currently active) — harmless, the next poll iteration
            // will see the terminal status anyway.
            let _ = agent_run_commands::stop_agent_run(app.clone(), agent_run_id.to_string()).await;
            stop_sent = true;
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Marks every `backlog` task `cancelled` and the mission itself `stopped`
/// — the honest terminal state for "the user stopped this mission", as
/// opposed to `failed` (misleading: nothing necessarily went wrong) or
/// silently leaving remaining tasks in `backlog` (misleading: they'd be
/// indistinguishable from a mission that was simply never started).
async fn stop_mission_now(app: &AppHandle, mission_id: &str) -> AppResult<()> {
    let tasks = {
        let conn = get_conn(app)?;
        tasks_repo::list_for_mission(&conn, mission_id)?
    };
    for task in tasks.iter().filter(|t| t.status == TaskStatus::Backlog) {
        {
            let conn = get_conn(app)?;
            tasks_repo::update_status(&conn, &task.id, TaskStatus::Cancelled)?;
        }
        events::mission_task_updated(app, mission_id, &task.id, TaskStatus::Cancelled, None)?;
    }

    {
        let conn = get_conn(app)?;
        missions_repo::mark_stopped(&conn, mission_id)?;
    }
    events::mission_status_changed(app, mission_id, MissionStatus::Stopped)?;
    Ok(())
}

/// Called once nothing is `backlog`-and-ready and nothing is running: the
/// mission has run to its natural end. `completed` only if every task
/// reached `done` (vacuously true for a zero-task plan); otherwise `failed`,
/// with an `error_message` spelling out exactly which tasks
/// succeeded/failed/were blocked, so a partial success is never collapsed
/// into a bare "failed" with no detail.
async fn finalize_natural_completion(app: &AppHandle, mission_id: &str) -> AppResult<()> {
    let tasks = {
        let conn = get_conn(app)?;
        tasks_repo::list_for_mission(&conn, mission_id)?
    };

    let total = tasks.len();
    let done = tasks.iter().filter(|t| t.status == TaskStatus::Done).count();

    if done == total {
        {
            let conn = get_conn(app)?;
            missions_repo::mark_completed(&conn, mission_id)?;
        }
        events::mission_status_changed(app, mission_id, MissionStatus::Completed)?;
        return Ok(());
    }

    let failed: Vec<&str> = tasks.iter().filter(|t| t.status == TaskStatus::Failed).map(|t| t.title.as_str()).collect();
    let blocked: Vec<&str> = tasks.iter().filter(|t| t.status == TaskStatus::Blocked).map(|t| t.title.as_str()).collect();
    let join_or_none = |items: &[&str]| if items.is_empty() { "none".to_string() } else { items.join(", ") };
    let message = format!(
        "{done} of {total} task(s) completed. Failed: {}. Blocked: {}.",
        join_or_none(&failed),
        join_or_none(&blocked)
    );

    {
        let conn = get_conn(app)?;
        missions_repo::mark_failed(&conn, mission_id, &message)?;
    }
    events::mission_status_changed(app, mission_id, MissionStatus::Failed)?;
    Ok(())
}

async fn run_mission_inner(app: &AppHandle, mission_id: &str, cancel: &CancellationToken) -> AppResult<()> {
    {
        let conn = get_conn(app)?;
        missions_repo::mark_running(&conn, mission_id)?;
    }
    events::mission_status_changed(app, mission_id, MissionStatus::Running)?;

    loop {
        if cancel.is_cancelled() {
            return stop_mission_now(app, mission_id).await;
        }

        let tasks = {
            let conn = get_conn(app)?;
            tasks_repo::list_for_mission(&conn, mission_id)?
        };
        let plan = evaluate(&tasks);

        if !plan.newly_blocked.is_empty() {
            for (task_id, reason) in &plan.newly_blocked {
                {
                    let conn = get_conn(app)?;
                    tasks_repo::update_status(&conn, task_id, TaskStatus::Blocked)?;
                }
                events::mission_task_updated(app, mission_id, task_id, TaskStatus::Blocked, Some(reason))?;
            }
            // Re-evaluate against the fresh state rather than reasoning
            // about which previously-`waiting` tasks might now also be
            // blocked transitively — cheap, and keeps this loop's logic to
            // one code path.
            continue;
        }

        let Some(next_task_id) = plan.ready.first().cloned() else {
            return finalize_natural_completion(app, mission_id).await;
        };

        if cancel.is_cancelled() {
            return stop_mission_now(app, mission_id).await;
        }

        let task = tasks
            .iter()
            .find(|t| t.id == next_task_id)
            .cloned()
            .expect("plan.ready ids are drawn from this same `tasks` snapshot");

        {
            let conn = get_conn(app)?;
            tasks_repo::update_status(&conn, &task.id, TaskStatus::InProgress)?;
        }
        events::mission_task_updated(app, mission_id, &task.id, TaskStatus::InProgress, None)?;

        let agent_run_id = start_task_execution(app, &task).await?;
        let run_status = poll_until_terminal(app, &agent_run_id, cancel).await?;
        let outcome = task_outcome_for_run(run_status, cancel.is_cancelled());
        let new_status = match outcome {
            TaskOutcome::Done => TaskStatus::Done,
            TaskOutcome::Failed => TaskStatus::Failed,
            TaskOutcome::Cancelled => TaskStatus::Cancelled,
        };

        {
            let conn = get_conn(app)?;
            tasks_repo::update_status(&conn, &task.id, new_status)?;
        }
        events::mission_task_updated(app, mission_id, &task.id, new_status, None)?;
    }
}

/// Drives `mission_id` from `approved`/`running` to a terminal mission
/// status. `cancel` is the same `CancellationToken`
/// `commands::mission_commands::start_mission` already registered in
/// `AppState.active_missions` before spawning this — passed in (not created
/// here) for the same reason `agent::tool_loop::run_agent_loop` takes one:
/// no window between "the command returned `Ok`" and "a `stop_mission` call
/// would actually find a token to cancel".
pub async fn run_mission(app: AppHandle, mission_id: String, cancel: CancellationToken) {
    let _guard = ActiveMissionGuard { app: app.clone(), mission_id: mission_id.clone() };

    if let Err(e) = run_mission_inner(&app, &mission_id, &cancel).await {
        // A hard, unrecoverable error outside the per-task handling above
        // (e.g. the DB became unreachable, or a task/mission row vanished
        // mid-run). Best-effort mark the mission failed so it doesn't sit
        // `running` forever.
        let msg = e.to_string();
        if let Ok(conn) = get_conn(&app) {
            let _ = missions_repo::mark_failed(&conn, &mission_id, &msg);
        }
        let _ = events::mission_status_changed(&app, &mission_id, MissionStatus::Failed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::TaskPriority;

    /// Builds a minimal in-memory `Task` for graph-logic tests — only the
    /// fields `evaluate`/`classify_task` actually look at (`id`, `status`,
    /// `position`, `depends_on_task_id`) vary between tests; everything else
    /// is a fixed placeholder.
    fn task(id: &str, status: TaskStatus, position: i64, depends_on: Option<&str>) -> Task {
        Task {
            id: id.to_string(),
            project_id: "p1".to_string(),
            mission_id: Some("m1".to_string()),
            title: format!("Task {id}"),
            description: None,
            status,
            priority: TaskPriority::Medium,
            position,
            depends_on_task_id: depends_on.map(str::to_string),
            agent_type: Some("backend".to_string()),
            agent_run_id: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // -- linear chain ----------------------------------------------------

    #[test]
    fn linear_chain_only_the_head_is_ready_at_the_start() {
        // A(backlog) -> B(backlog, dep A) -> C(backlog, dep B)
        let tasks =
            vec![task("a", TaskStatus::Backlog, 0, None), task("b", TaskStatus::Backlog, 1, Some("a")), task("c", TaskStatus::Backlog, 2, Some("b"))];

        let eval = evaluate(&tasks);
        assert_eq!(eval.ready, vec!["a".to_string()]);
        assert!(eval.newly_blocked.is_empty());
    }

    #[test]
    fn linear_chain_progresses_one_task_at_a_time_as_each_completes() {
        // A(done) -> B(backlog, ready) -> C(backlog, still waiting on B).
        let tasks =
            vec![task("a", TaskStatus::Done, 0, None), task("b", TaskStatus::Backlog, 1, Some("a")), task("c", TaskStatus::Backlog, 2, Some("b"))];
        let eval = evaluate(&tasks);
        assert_eq!(eval.ready, vec!["b".to_string()], "only B should be ready — C still waits on B");
        assert!(eval.newly_blocked.is_empty());

        // A(done) -> B(done) -> C(backlog, now ready).
        let tasks =
            vec![task("a", TaskStatus::Done, 0, None), task("b", TaskStatus::Done, 1, Some("a")), task("c", TaskStatus::Backlog, 2, Some("b"))];
        let eval = evaluate(&tasks);
        assert_eq!(eval.ready, vec!["c".to_string()]);
    }

    // -- diamond / converging dependencies --------------------------------

    #[test]
    fn diamond_shaped_graph_fans_out_then_reconverges() {
        // A single `depends_on_task_id` column models one predecessor per
        // task (see `orchestrator::planner`'s own doc comment on this same
        // limitation), so a "diamond" here is: A done -> B and C both
        // become ready (fan-out); D depends only on B (the modelable half
        // of the reconvergence) and stays blocked-on-B until B finishes,
        // regardless of C's state.
        let tasks = vec![
            task("a", TaskStatus::Done, 0, None),
            task("b", TaskStatus::Backlog, 1, Some("a")),
            task("c", TaskStatus::Backlog, 2, Some("a")),
            task("d", TaskStatus::Backlog, 3, Some("b")),
        ];
        let eval = evaluate(&tasks);
        assert_eq!(eval.ready, vec!["b".to_string(), "c".to_string()], "both branches of the fan-out should be ready together");
        assert!(!eval.ready.contains(&"d".to_string()), "D depends on B, which hasn't finished yet");

        // B finishes -> D becomes ready; C's own state doesn't gate D.
        let tasks = vec![
            task("a", TaskStatus::Done, 0, None),
            task("b", TaskStatus::Done, 1, Some("a")),
            task("c", TaskStatus::Backlog, 2, Some("a")),
            task("d", TaskStatus::Backlog, 3, Some("b")),
        ];
        let eval = evaluate(&tasks);
        assert_eq!(eval.ready, vec!["c".to_string(), "d".to_string()]);
    }

    // -- cycle -------------------------------------------------------------

    #[test]
    fn dependency_cycle_is_detected_and_blocks_without_looping() {
        // A -> B -> C -> A. None of these should ever classify as `Ready`
        // or `Waiting` forever — each is `Blocked` with a cycle reason.
        let tasks =
            vec![task("a", TaskStatus::Backlog, 0, Some("c")), task("b", TaskStatus::Backlog, 1, Some("a")), task("c", TaskStatus::Backlog, 2, Some("b"))];

        let eval = evaluate(&tasks);
        assert!(eval.ready.is_empty());
        let blocked_ids: HashSet<&str> = eval.newly_blocked.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(blocked_ids, HashSet::from(["a", "b", "c"]));
        for (_, reason) in &eval.newly_blocked {
            assert!(reason.contains("cycle"), "reason should say why: {reason}");
        }
    }

    #[test]
    fn self_dependency_is_treated_as_a_cycle() {
        let tasks = vec![task("a", TaskStatus::Backlog, 0, Some("a"))];
        let eval = evaluate(&tasks);
        assert!(eval.ready.is_empty());
        assert_eq!(eval.newly_blocked.len(), 1);
        assert_eq!(eval.newly_blocked[0].0, "a");
    }

    // -- failed dependency ---------------------------------------------

    #[test]
    fn task_blocked_by_a_failed_dependency() {
        let tasks = vec![task("a", TaskStatus::Failed, 0, None), task("b", TaskStatus::Backlog, 1, Some("a"))];
        let eval = evaluate(&tasks);
        assert!(eval.ready.is_empty());
        assert_eq!(eval.newly_blocked.len(), 1);
        assert_eq!(eval.newly_blocked[0].0, "b");
        assert!(eval.newly_blocked[0].1.contains("did not complete successfully"));
    }

    #[test]
    fn task_blocked_by_an_already_blocked_dependency_propagates() {
        // A(failed) -> B(blocked, from a previous pass) -> C(backlog, dep B).
        let tasks = vec![
            task("a", TaskStatus::Failed, 0, None),
            task("b", TaskStatus::Blocked, 1, Some("a")),
            task("c", TaskStatus::Backlog, 2, Some("b")),
        ];
        let eval = evaluate(&tasks);
        assert!(eval.ready.is_empty());
        assert_eq!(eval.newly_blocked.len(), 1, "only C should be newly blocked this pass — B already is");
        assert_eq!(eval.newly_blocked[0].0, "c");
    }

    #[test]
    fn task_blocked_by_a_cancelled_dependency() {
        let tasks = vec![task("a", TaskStatus::Cancelled, 0, None), task("b", TaskStatus::Backlog, 1, Some("a"))];
        let eval = evaluate(&tasks);
        assert_eq!(eval.newly_blocked.len(), 1);
        assert_eq!(eval.newly_blocked[0].0, "b");
    }

    // -- multiple independently-ready tasks -------------------------------

    #[test]
    fn multiple_independently_ready_tasks_are_all_returned() {
        // Three tasks, no dependencies between any of them — all three are
        // ready simultaneously. This milestone only *starts* one at a time,
        // but the readiness computation itself must not silently drop the
        // others.
        let tasks = vec![task("a", TaskStatus::Backlog, 2, None), task("b", TaskStatus::Backlog, 0, None), task("c", TaskStatus::Backlog, 1, None)];
        let eval = evaluate(&tasks);
        assert_eq!(eval.ready, vec!["b".to_string(), "c".to_string(), "a".to_string()], "ready ids should be ordered by position");
    }

    // -- simple no-dependency case -----------------------------------------

    #[test]
    fn task_with_no_dependency_is_immediately_ready() {
        let tasks = vec![task("a", TaskStatus::Backlog, 0, None)];
        let eval = evaluate(&tasks);
        assert_eq!(eval.ready, vec!["a".to_string()]);
        assert!(eval.newly_blocked.is_empty());
    }

    #[test]
    fn non_backlog_tasks_are_never_classified() {
        // done/in_progress/todo tasks shouldn't show up in either list, even
        // if they happen to have a `depends_on_task_id` set.
        let tasks = vec![
            task("a", TaskStatus::Done, 0, None),
            task("b", TaskStatus::InProgress, 1, Some("a")),
            task("c", TaskStatus::Todo, 2, None),
        ];
        let eval = evaluate(&tasks);
        assert!(eval.ready.is_empty());
        assert!(eval.newly_blocked.is_empty());
    }

    #[test]
    fn empty_task_set_evaluates_cleanly() {
        let eval = evaluate(&[]);
        assert!(eval.ready.is_empty());
        assert!(eval.newly_blocked.is_empty());
    }

    // -- task_outcome_for_run ----------------------------------------------

    #[test]
    fn outcome_completed_run_is_done() {
        assert_eq!(task_outcome_for_run(AgentRunStatus::Completed, false), TaskOutcome::Done);
        assert_eq!(task_outcome_for_run(AgentRunStatus::Completed, true), TaskOutcome::Done);
    }

    #[test]
    fn outcome_failed_run_is_failed_regardless_of_mission_stop() {
        assert_eq!(task_outcome_for_run(AgentRunStatus::Failed, false), TaskOutcome::Failed);
        assert_eq!(task_outcome_for_run(AgentRunStatus::Failed, true), TaskOutcome::Failed);
    }

    #[test]
    fn outcome_stopped_run_splits_on_whether_the_mission_requested_it() {
        assert_eq!(
            task_outcome_for_run(AgentRunStatus::Stopped, true),
            TaskOutcome::Cancelled,
            "stop_mission caused this — the task itself didn't fail"
        );
        assert_eq!(
            task_outcome_for_run(AgentRunStatus::Stopped, false),
            TaskOutcome::Failed,
            "stopped for some other reason (max iterations, a direct stop) — treat as failed, no partial credit"
        );
    }
}

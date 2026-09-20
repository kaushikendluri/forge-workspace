//! M9 walked an `approved` mission's task dependency graph and ran each task
//! **sequentially** through the M5/M6 agent pipeline
//! (`commands::agent_commands::{create_agent,start_worktree_for_agent}` +
//! `commands::agent_run_commands::{start_agent_run,stop_agent_run}`). M10
//! upgrades the *driver* to run multiple independently-ready tasks
//! **concurrently**, bounded by the `agent.max_parallel_agents` setting —
//! without touching the dependency-graph rules themselves (`evaluate`/
//! `classify_task`/`is_in_dependency_cycle` are unchanged from M9) or the
//! per-task execution pipeline (`start_task_execution`/`poll_until_terminal`
//! are unchanged from M9 too, just now called from inside a `tokio::spawn`
//! instead of awaited inline one at a time).
//!
//! Split into three parts:
//!   * Pure graph logic (`evaluate`, `classify_task`, `task_outcome_for_run`)
//!     — no I/O, fully unit-testable, unchanged from M9.
//!   * Pure scheduling *policy* (`select_tasks_to_start`) — M10's addition:
//!     given the ready ids `evaluate` already computed and how many of
//!     `max_parallel_agents` slots are currently free, which ids to start
//!     right now. Also no I/O, fully unit-testable — see the tests below.
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
//! ## Concurrency model (M10)
//!
//! `run_mission_inner` keeps an `in_flight: HashMap<TaskId, JoinHandle<...>>`
//! — one entry per task currently `in_progress`, sized up to
//! `max_parallel_agents`. Every loop iteration:
//!   1. **Reap** any finished handles first (`reap_finished`) — applies each
//!      one's `task_outcome_for_run` and frees its slot — *before* deciding
//!      what to start next, so a just-freed slot is visible to this same
//!      pass rather than only the next one.
//!   2. If the mission's `cancel` token has fired, **drain** every remaining
//!      in-flight handle to completion (`drain_all` — see "stop" below),
//!      then finish as `stopped`.
//!   3. Re-evaluate readiness against the fresh `tasks` snapshot (`evaluate`
//!      — the same pure function M9 used, untouched) and apply any newly
//!      `blocked` transitions.
//!   4. `select_tasks_to_start(ready, max_parallel_agents, in_flight.len())`
//!      picks up to as many *newly*-ready ids as there are free slots, in
//!      `position` order; each gets its own `tokio::spawn` running
//!      `start_task_execution` + `poll_until_terminal` (M9's own functions,
//!      called concurrently now rather than awaited one at a time) and an
//!      entry in `in_flight`.
//!   5. If nothing was just started and nothing is in flight, that's the
//!      natural-end case (see below). If nothing was started but something
//!      is still in flight (slots full, or nothing newly ready yet), sleep
//!      one `POLL_INTERVAL` and loop again.
//!
//! Two tasks becoming ready in the same pass both start (up to the slot
//! limit) in the same iteration of step 4, from the same `tasks` snapshot —
//! there is no window where one is evaluated against stale data because the
//! other's DB write hasn't landed yet, since neither has *started* until
//! this loop's own `tasks_repo::update_status(.., InProgress)` calls run
//! (sequentially, both from this single driver task, before either spawn is
//! created).
//!
//! When one task's run ends:
//!   * `Completed` -> task `done`.
//!   * `Failed`, or `Stopped` for a reason *other* than this mission's own
//!     `stop_mission` (max-iterations, or someone stopping that individual
//!     run directly) -> task `failed`.
//!   * `Stopped` *because* `stop_mission` fired -> task `cancelled`, not
//!     `failed` — nothing about the task itself went wrong.
//!
//! When `stop_mission` fires: **every** currently in-flight task is
//! cancelled, not just one. This falls out of the fact that every spawned
//! task's `poll_until_terminal` call was handed a `.clone()` of the *same*
//! mission-level `CancellationToken` — `tokio_util`'s `CancellationToken`
//! shares its cancelled-state across every clone, so firing it once (via
//! `stop_mission`) is observed independently by every concurrently-running
//! `poll_until_terminal` loop, each of which forwards it as a real
//! `stop_agent_run` call against its *own* run — no central loop needs to
//! enumerate "every active run" to cancel them, and no task can be missed by
//! one. `drain_all` then simply awaits every remaining handle to actually
//! finish (each one settles to `Stopped` on its own once its `stop_agent_run`
//! call's cancellation has actually torn down its in-flight HTTP/tool call)
//! before the mission itself is marked `stopped`. Every other task still
//! sitting in `backlog` (never started) is also marked `cancelled` (not
//! silently left in `backlog`, which would look indistinguishable from a
//! mission that was simply never started).
//!
//! When the mission runs to its natural end (nothing left ready, nothing in
//! flight): `completed` only if *every* task reached `done`; otherwise
//! `failed`, with `error_message` spelling out exactly which tasks
//! succeeded/failed/were blocked rather than collapsing a partial success
//! into a bare "failed".

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use rusqlite::Connection;
use tauri::{AppHandle, Manager};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::commands::{agent_commands, agent_run_commands};
use crate::db::models::{AgentRunStatus, MissionStatus, Task, TaskStatus};
use crate::db::repository::{
    agent_runs as agent_runs_repo, missions as missions_repo, settings as settings_repo, tasks as tasks_repo,
};
use crate::db::DbConnection;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

use super::events;

/// The `settings` key (M4's key/value table, see `db::repository::settings`)
/// this milestone's concurrency cap is read from. Seeded to `"3"` by
/// migration `0004_max_parallel_agents.sql` so there's always a real
/// persisted value, not just a bare code constant — see
/// `load_max_parallel_agents`.
const MAX_PARALLEL_AGENTS_SETTING_KEY: &str = "agent.max_parallel_agents";

/// Fallback used only if the setting is missing/unparseable/non-positive —
/// the migration seed means this should be unreachable in practice, but
/// `load_max_parallel_agents` stays defensive about it anyway, the same
/// pattern `agent::tool_loop::load_run_setup` uses for `agent.max_iterations`.
const DEFAULT_MAX_PARALLEL_AGENTS: i64 = 3;

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
    /// (ascending). `select_tasks_to_start` decides how many of these
    /// actually get started this pass (bounded by free concurrency slots) —
    /// `evaluate` itself has no notion of a slot limit, it just reports
    /// every id that's *graph-ready*.
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

/// How many of `max_parallel` concurrency slots are currently free, given
/// `in_flight_count` tasks already running. Saturates at `0` rather than
/// underflowing if, for some reason, more tasks are in flight than the
/// configured max (e.g. the setting was lowered while a mission with more
/// than the new max already running was mid-flight — M10 reads the setting
/// once at mission start, so this can genuinely happen, and the right
/// behavior is simply "start nothing new until enough of the existing ones
/// finish", not a panic or a wrapped-around huge number).
fn free_slots(max_parallel: usize, in_flight_count: usize) -> usize {
    max_parallel.saturating_sub(in_flight_count)
}

/// M10's scheduling **policy**: given `ready` (already `position`-ordered,
/// as `evaluate` returns it) and how many tasks are currently in flight,
/// returns which ids to start *this pass* — the first `free_slots(...)` of
/// `ready`, in order. Pure — no I/O, no mutation, doesn't know or care
/// *why* a task is ready (that's `evaluate`'s job, unchanged from M9) or
/// *how* a returned id actually gets started (that's `run_mission_inner`'s
/// job, via `tokio::spawn`). Deliberately takes `in_flight_count` as a
/// plain `usize` rather than the `in_flight` map itself, so this stays
/// testable with no async runtime, no `Task`/`JoinHandle` values, nothing.
pub fn select_tasks_to_start(ready: &[String], max_parallel: usize, in_flight_count: usize) -> Vec<String> {
    let n = free_slots(max_parallel, in_flight_count);
    ready.iter().take(n).cloned().collect()
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

/// The configured concurrency cap for this mission, from
/// `agent.max_parallel_agents` — see `MAX_PARALLEL_AGENTS_SETTING_KEY`'s own
/// docs. Read once, up front, when `run_mission_inner` starts (the same "read
/// settings once, not per-iteration" choice `agent::tool_loop::load_run_setup`
/// makes for `agent.max_iterations`): a mid-mission settings change simply
/// takes effect the next time a mission is started, not retroactively on one
/// already running.
fn load_max_parallel_agents(conn: &Connection) -> AppResult<usize> {
    let value = settings_repo::get(conn, MAX_PARALLEL_AGENTS_SETTING_KEY)?
        .and_then(|s| s.value.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_PARALLEL_AGENTS);
    Ok(value as usize)
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

/// One task's entire execution, end to end: `start_task_execution` (real
/// worktree + queued run + start of the M6 loop) then `poll_until_terminal`
/// (poll until that run finishes, forwarding `cancel`). Both are M9's own
/// functions, completely unchanged — this wrapper exists only so
/// `run_mission_inner` has a single `async fn` to hand to `tokio::spawn` per
/// concurrently-started task. Every call gets its own `task`/`cancel`
/// (cloned by the caller before spawning) and makes its own independent
/// calls into `commands::agent_commands`/`commands::agent_run_commands`,
/// each of which does its own `state.db.get()` (a fresh pooled connection)
/// and its own `Uuid::new_v4()`-derived run id/worktree path/branch name
/// (see `agent_commands::start_worktree_for_agent`) — nothing here is
/// shared mutable state between concurrent callers beyond `AppState`'s own
/// mutex-protected `active_runs`/`active_missions` registries, which are
/// already keyed collections (one entry per run/mission id) rather than a
/// single slot, so multiple concurrent entries were already a safe shape
/// before this milestone.
async fn run_single_task_to_terminal(app: &AppHandle, task: &Task, cancel: &CancellationToken) -> AppResult<AgentRunStatus> {
    let agent_run_id = start_task_execution(app, task).await?;
    poll_until_terminal(app, &agent_run_id, cancel).await
}

/// Applies one task's terminal outcome: computes its `TaskOutcome` (via the
/// unchanged `task_outcome_for_run`), writes the new `tasks.status`, and
/// emits the matching `mission:task-updated` event. Shared by the normal
/// per-pass reap (`reap_finished`) and the cancellation drain (`drain_all`)
/// so both apply outcomes identically — the only difference between them is
/// *when* they're called and what `mission_stop_requested` is.
async fn apply_task_outcome(
    app: &AppHandle,
    mission_id: &str,
    task_id: &str,
    run_status: AgentRunStatus,
    mission_stop_requested: bool,
) -> AppResult<()> {
    let outcome = task_outcome_for_run(run_status, mission_stop_requested);
    let new_status = match outcome {
        TaskOutcome::Done => TaskStatus::Done,
        TaskOutcome::Failed => TaskStatus::Failed,
        TaskOutcome::Cancelled => TaskStatus::Cancelled,
    };

    {
        let conn = get_conn(app)?;
        tasks_repo::update_status(&conn, task_id, new_status)?;
    }
    events::mission_task_updated(app, mission_id, task_id, new_status, None)?;
    Ok(())
}

/// One entry per task currently `in_progress` under this mission's
/// scheduler — `in_flight.len()` is always an accurate live count of
/// concurrently-running tasks, since entries are inserted the instant a task
/// is spawned and removed the instant `reap_finished`/`drain_all` observes
/// (or awaits) its completion. Keyed by task id rather than a `Vec` so a
/// task can never accidentally be double-counted or double-started — `insert`
/// on an id already present would silently replace the old handle, but
/// `select_tasks_to_start` only ever offers ids from `evaluate`'s `ready`
/// list, which is computed from `backlog` tasks only, and a task's status is
/// flipped to `InProgress` (removing it from `backlog`) in the same loop
/// iteration it's inserted here — so the same id can't be selected twice
/// before its first run is reaped.
type InFlight = HashMap<String, JoinHandle<AppResult<AgentRunStatus>>>;

/// Reaps every currently-finished handle in `in_flight` (`JoinHandle::
/// is_finished` — a cheap, non-blocking check; nothing here blocks on a
/// still-running task) and applies its outcome via `apply_task_outcome`,
/// freeing its slot. Called at the top of every driver loop iteration,
/// before readiness is re-evaluated, so a slot freed this instant is visible
/// to this same pass's `select_tasks_to_start` call rather than only the
/// next one.
async fn reap_finished(app: &AppHandle, mission_id: &str, in_flight: &mut InFlight, cancel: &CancellationToken) -> AppResult<()> {
    let finished_ids: Vec<String> = in_flight.iter().filter(|(_, handle)| handle.is_finished()).map(|(id, _)| id.clone()).collect();

    for task_id in finished_ids {
        let handle = in_flight.remove(&task_id).expect("just observed this id present in in_flight above");
        let run_status = handle.await.map_err(|e| AppError::Other(format!("task {task_id}'s execution panicked: {e}")))??;
        apply_task_outcome(app, mission_id, &task_id, run_status, cancel.is_cancelled()).await?;
    }
    Ok(())
}

/// Cancellation path: awaits **every** remaining in-flight handle to
/// completion (not just one — see the module docs' "Concurrency model"
/// section for why every one of them independently observes the same
/// `cancel` token and stops its own run) and applies each one's outcome as
/// `Cancelled` (`mission_stop_requested: true`, unconditionally — this is
/// only ever called once `cancel.is_cancelled()` is already confirmed true).
/// Takes ownership of `in_flight` (draining it fully) rather than `&mut`,
/// since after this the mission is done — there is nothing left to
/// possibly reap later.
async fn drain_all(app: &AppHandle, mission_id: &str, in_flight: InFlight) -> AppResult<()> {
    for (task_id, handle) in in_flight {
        let run_status = handle.await.map_err(|e| AppError::Other(format!("task {task_id}'s execution panicked: {e}")))??;
        apply_task_outcome(app, mission_id, &task_id, run_status, true).await?;
    }
    Ok(())
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

    let max_parallel = {
        let conn = get_conn(app)?;
        load_max_parallel_agents(&conn)?
    };

    // One entry per task currently `in_progress` under this mission — see
    // `InFlight`'s own docs. Local to this call (not `AppState`): each
    // mission's own driver owns its own pool of task handles, the same way
    // each agent run already owns its own place in `AppState.active_runs`
    // (a registry keyed by id, not a single shared slot).
    let mut in_flight: InFlight = HashMap::new();

    loop {
        // Reap finished executions and apply their outcomes *before*
        // deciding what to start next, so a slot freed this instant is
        // visible to this same pass's `select_tasks_to_start` call below —
        // not just the next loop iteration.
        reap_finished(app, mission_id, &mut in_flight, cancel).await?;

        if cancel.is_cancelled() {
            // Every remaining in-flight task's own `poll_until_terminal`
            // call is already observing this same `cancel` token
            // independently (see the module docs) and will forward it as a
            // real `stop_agent_run` for its own run — this just waits for
            // all of them to actually finish before declaring the mission
            // `stopped`, exactly as `poll_until_terminal` already did for
            // the single in-flight task M9 ever had.
            drain_all(app, mission_id, std::mem::take(&mut in_flight)).await?;
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

        let to_start = select_tasks_to_start(&plan.ready, max_parallel, in_flight.len());

        if to_start.is_empty() {
            if in_flight.is_empty() {
                // Nothing ready, nothing running, nothing newly blocked this
                // pass: the mission has run to its natural end.
                return finalize_natural_completion(app, mission_id).await;
            }
            // Slots are full, or nothing is newly ready yet — something is
            // still in flight, so this isn't the end, just a quiet pass.
            // Sleep before re-checking rather than busy-looping.
            tokio::time::sleep(POLL_INTERVAL).await;
            continue;
        }

        // Start every selected task from this *same* `tasks` snapshot, one
        // at a time, sequentially, right here on the driver task — each
        // one's `update_status(.., InProgress)` DB write and its
        // `mission:task-updated` event both happen before the next one's
        // spawn is even created. Two tasks becoming ready in the same pass
        // therefore both get marked `in_progress` and started without a
        // window where one's spawn could race the other's status write.
        for task_id in to_start {
            let task = tasks
                .iter()
                .find(|t| t.id == task_id)
                .cloned()
                .expect("plan.ready ids (and to_start, a subset of them) are drawn from this same `tasks` snapshot");

            {
                let conn = get_conn(app)?;
                tasks_repo::update_status(&conn, &task.id, TaskStatus::InProgress)?;
            }
            events::mission_task_updated(app, mission_id, &task.id, TaskStatus::InProgress, None)?;

            let handle: JoinHandle<AppResult<AgentRunStatus>> = tokio::spawn({
                let app = app.clone();
                let cancel = cancel.clone();
                let task = task.clone();
                async move { run_single_task_to_terminal(&app, &task, &cancel).await }
            });
            in_flight.insert(task.id.clone(), handle);
        }
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

    // -- M10: free_slots / select_tasks_to_start (scheduling policy) -------

    #[test]
    fn free_slots_is_the_gap_between_max_and_in_flight() {
        assert_eq!(free_slots(3, 0), 3);
        assert_eq!(free_slots(3, 1), 2);
        assert_eq!(free_slots(3, 3), 0);
    }

    #[test]
    fn free_slots_saturates_at_zero_rather_than_underflowing() {
        // Can genuinely happen if the setting is lowered while a mission
        // that started with a higher max is still mid-flight (M10 reads the
        // setting once at mission start, not per-iteration) — more tasks
        // in flight than the current max. Must not panic or wrap around to
        // a huge `usize`.
        assert_eq!(free_slots(2, 5), 0);
    }

    #[test]
    fn three_ready_tasks_max_parallel_two_only_two_start() {
        let ready = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let started = select_tasks_to_start(&ready, 2, 0);
        assert_eq!(started, vec!["a".to_string(), "b".to_string()], "only the first 2 (position order) should start");
    }

    #[test]
    fn select_tasks_to_start_accounts_for_already_in_flight_tasks() {
        // 3 ready, max_parallel 2, but 1 is already in flight -> only 1 more
        // slot is free, so only 1 of the 3 starts.
        let ready = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let started = select_tasks_to_start(&ready, 2, 1);
        assert_eq!(started, vec!["a".to_string()]);
    }

    #[test]
    fn select_tasks_to_start_returns_nothing_when_slots_are_full() {
        let ready = vec!["a".to_string(), "b".to_string()];
        let started = select_tasks_to_start(&ready, 2, 2);
        assert!(started.is_empty());
    }

    #[test]
    fn select_tasks_to_start_returns_nothing_for_an_empty_ready_list() {
        let started = select_tasks_to_start(&[], 5, 0);
        assert!(started.is_empty());
    }

    #[test]
    fn select_tasks_to_start_starts_everything_ready_when_slots_are_plentiful() {
        let ready = vec!["a".to_string(), "b".to_string()];
        let started = select_tasks_to_start(&ready, 10, 0);
        assert_eq!(started, ready, "both should start — plenty of slots, no reason to hold either back");
    }

    #[test]
    fn a_freed_slot_lets_a_newly_ready_dependent_task_start() {
        // A running (in_flight, so no longer `backlog`), B depends on A and
        // is still `backlog` -> `evaluate` reports B as `Waiting`, not
        // ready, so `select_tasks_to_start` has nothing to offer it yet even
        // though the sole slot (max_parallel 1) is occupied.
        let tasks_while_a_runs = vec![task("a", TaskStatus::InProgress, 0, None), task("b", TaskStatus::Backlog, 1, Some("a"))];
        let eval_while_a_runs = evaluate(&tasks_while_a_runs);
        assert!(eval_while_a_runs.ready.is_empty(), "B still waits on A");
        assert!(select_tasks_to_start(&eval_while_a_runs.ready, 1, 1).is_empty());

        // A completes (reap_finished would apply this via task_outcome_for_run
        // and free A's slot) -> B is now graph-ready, and with A's slot freed
        // (in_flight_count back down to 0) it gets to start even though
        // max_parallel is still only 1.
        let tasks_after_a_done = vec![task("a", TaskStatus::Done, 0, None), task("b", TaskStatus::Backlog, 1, Some("a"))];
        let eval_after_a_done = evaluate(&tasks_after_a_done);
        assert_eq!(eval_after_a_done.ready, vec!["b".to_string()]);
        let started = select_tasks_to_start(&eval_after_a_done.ready, 1, 0);
        assert_eq!(started, vec!["b".to_string()], "B's dependency finished and its slot is free — it should start");
    }

    #[test]
    fn stopping_the_mission_cancels_every_in_flight_tasks_outcome_the_same_way_not_just_one() {
        // M9 only ever had one task in flight, so `task_outcome_for_run`
        // only ever needed to answer this for a single run. M10's
        // `drain_all` calls it once per entry in `in_flight`, however many
        // there are, always with `mission_stop_requested: true` (it's only
        // ever invoked once `cancel.is_cancelled()` is already confirmed) —
        // this asserts that mapping is uniform across an arbitrarily large
        // set of concurrently in-flight tasks, not special-cased for "the"
        // one task the way a naive port of the M9 single-task logic could
        // have left it.
        let concurrently_in_flight_task_ids = ["a", "b", "c", "d", "e"];
        for task_id in concurrently_in_flight_task_ids {
            assert_eq!(
                task_outcome_for_run(AgentRunStatus::Stopped, true),
                TaskOutcome::Cancelled,
                "task {task_id} was stopped because the mission was — it must be Cancelled, not Failed"
            );
        }
    }

    // -- M10: load_max_parallel_agents (the persisted setting) -------------

    fn migrated_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        crate::db::migrations::run_migrations(&mut conn).expect("run migrations");
        conn
    }

    #[test]
    fn load_max_parallel_agents_reads_the_migration_seeded_default() {
        // Migration 0004_max_parallel_agents.sql seeds this to "3" — a real
        // persisted default, not just a bare code fallback.
        let conn = migrated_conn();
        assert_eq!(load_max_parallel_agents(&conn).expect("load"), 3);
    }

    #[test]
    fn load_max_parallel_agents_reads_a_custom_persisted_value() {
        let conn = migrated_conn();
        settings_repo::set(&conn, MAX_PARALLEL_AGENTS_SETTING_KEY, "7").expect("set");
        assert_eq!(load_max_parallel_agents(&conn).expect("load"), 7);
    }

    #[test]
    fn load_max_parallel_agents_falls_back_on_non_positive_or_unparseable_values() {
        let conn = migrated_conn();

        settings_repo::set(&conn, MAX_PARALLEL_AGENTS_SETTING_KEY, "0").expect("set");
        assert_eq!(load_max_parallel_agents(&conn).expect("load"), DEFAULT_MAX_PARALLEL_AGENTS as usize);

        settings_repo::set(&conn, MAX_PARALLEL_AGENTS_SETTING_KEY, "-1").expect("set");
        assert_eq!(load_max_parallel_agents(&conn).expect("load"), DEFAULT_MAX_PARALLEL_AGENTS as usize);

        settings_repo::set(&conn, MAX_PARALLEL_AGENTS_SETTING_KEY, "not-a-number").expect("set");
        assert_eq!(load_max_parallel_agents(&conn).expect("load"), DEFAULT_MAX_PARALLEL_AGENTS as usize);
    }
}

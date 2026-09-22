//! Rust structs mirroring the SQLite schema in
//! `migrations/0001_init.sql`, kept in 1:1 correspondence with the
//! TypeScript types in `src/types/db.ts`. `#[serde(rename_all = "camelCase")]`
//! matches the frontend's camelCase field naming over Tauri's JSON bridge.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: String,
    pub project_id: String,
    pub root_path: String,
    pub remote_url: Option<String>,
    pub default_branch: String,
    pub vcs_type: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Running,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub project_id: String,
    pub repository_id: String,
    pub name: String,
    pub status: AgentStatus,
    pub system_prompt: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStopReason {
    Completed,
    MaxIterations,
    UserStopped,
    Error,
    /// M13: the run's `agent.max_test_fix_attempts` cap for the self-healing
    /// test-fix cycle was exceeded — see `agent::test_fix::TestFixTracker`.
    /// Distinct from `Error`: this is not a failure of the run itself (the
    /// model may have been doing legitimate, unrelated work), just a
    /// deliberate stop so a human can look rather than letting the
    /// fail→fix→retest cycle run unbounded.
    TestFixBudgetExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub id: String,
    pub agent_id: String,
    pub workspace_id: Option<String>,
    pub task_prompt: String,
    pub model_id: String,
    pub status: AgentRunStatus,
    pub stop_reason: Option<AgentRunStopReason>,
    pub error_message: Option<String>,
    pub iteration_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    /// M13: the self-healing test-fix cycle's bounded retry counter — see
    /// `agent::test_fix::TestFixTracker`. Mirrored here from the in-memory
    /// tracker each time it changes, not incremented directly by SQL.
    pub test_fix_attempts: i64,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    Primary,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatus {
    Active,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub repository_id: String,
    pub agent_run_id: Option<String>,
    pub kind: WorkspaceKind,
    pub path: String,
    pub branch_name: String,
    pub base_branch: Option<String>,
    pub base_commit_sha: Option<String>,
    pub status: WorkspaceStatus,
    pub created_at: String,
    pub removed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Running,
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub agent_run_id: String,
    pub sequence_number: i64,
    pub tool_use_id: String,
    pub tool_name: String,
    pub input_json: String,
    pub output_json: Option<String>,
    pub status: ToolCallStatus,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEventType {
    RunStarted,
    ModelMessage,
    ToolCallStarted,
    ToolCallCompleted,
    RunCompleted,
    RunStopped,
    Error,
    /// M13: emitted specifically when a `run_tests` failure is detected
    /// (the start of a potential fix attempt) and again when a subsequent
    /// retest of that attempt completes (pass or still failing), plus once
    /// more if the run's `agent.max_test_fix_attempts` budget is exceeded —
    /// see `agent::test_fix::TestFixTracker`. Its `payload_json` carries
    /// `phase` (`"failed"` | `"retested"` | `"budget_exhausted"`), `attempt`,
    /// and — when it refers to one — the `testRunId` of the real
    /// `test_runs` row (M12) for that specific test run.
    TestFixCycle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    pub agent_run_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub event_type: ActivityEventType,
    pub payload_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Planned by a mission (M8) but not yet started — distinct from `Todo`
    /// so a mission's freshly-approved-but-not-yet-picked-up tasks read
    /// honestly rather than looking like manually triaged work.
    Backlog,
    Todo,
    InProgress,
    Done,
    /// M9: this task's agent run ended without succeeding, for a reason
    /// other than the mission itself being stopped (the agent gave up, hit
    /// max iterations, or a genuine error) — terminal.
    Failed,
    /// M9: permanently unable to run — its `depends_on_task_id` task ended
    /// `Failed`/`Blocked`/`Cancelled` instead of `Done`, or it's part of a
    /// dependency cycle (see `orchestrator::scheduler`). Terminal.
    Blocked,
    /// M9: the mission was stopped (`stop_mission`) before this task got a
    /// chance to run — distinct from `Failed`/`Blocked` since nothing about
    /// this task itself went wrong. Terminal.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project_id: String,
    /// The mission (M8) that proposed this task, if any — `None` for a task
    /// created directly (not via a mission plan).
    pub mission_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    /// Display order within its mission's plan (0-based). `0` for a task
    /// with no mission.
    pub position: i64,
    /// Another task (usually in the same mission) that must complete before
    /// this one can start, if any.
    pub depends_on_task_id: Option<String>,
    /// The plan's suggested specialist role for this task (e.g.
    /// `"frontend"`, `"backend"`, `"database"`, `"qa"`) — a free-form label,
    /// not a foreign key: Agent Skills don't exist until Phase 5.
    pub agent_type: Option<String>,
    pub agent_run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Planning,
    PlanReady,
    Approved,
    Running,
    Completed,
    Failed,
    /// M9: `stop_mission` was called while this mission was running —
    /// distinct from `Failed` (nothing necessarily went wrong; the user
    /// chose to stop) and `Completed` (not every task ran).
    Stopped,
}

/// A mission (M8): a plain-English objective, turned into a structured task
/// plan by one Anthropic Messages API call (`orchestrator::planner`), which
/// the user reviews and approves before anything executes. M9's
/// `orchestrator::scheduler` walks an `approved` mission's task graph and
/// runs each task through the real M5/M6 agent pipeline, moving the mission
/// to `Running`, then `Completed`/`Failed`/`Stopped`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mission {
    pub id: String,
    pub project_id: String,
    pub objective: String,
    pub status: MissionStatus,
    /// The raw `propose_plan` tool input, serialized, once the plan is
    /// ready — kept alongside the real `tasks` rows it produced as the
    /// original model output for reference/debugging.
    pub plan_json: Option<String>,
    /// Set when `status` is `failed` — the real planner error (e.g. "no API
    /// key configured", an Anthropic API error, a malformed plan), never a
    /// generic message.
    pub error_message: Option<String>,
    pub created_at: String,
    pub approved_at: Option<String>,
    pub completed_at: Option<String>,
}

/// M11: one agent-to-agent structured message within a mission, written by
/// the `send_message` tool (`agent::tools`) and read back by
/// `list_agent_messages` for Mission Control's messages panel and
/// `AgentDetail.tsx`'s activity stream. `to_agent_run_id` is `None` for a
/// mission-wide broadcast (no `to_task_title` given, or that task hasn't run
/// yet) — this is a persisted log, not a live chat, so it may also point at
/// a run that has already finished by the time it's read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessage {
    pub id: String,
    pub from_agent_run_id: String,
    pub to_agent_run_id: Option<String>,
    pub mission_id: String,
    pub subject: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub id: String,
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub is_default: bool,
    pub max_output_tokens: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    AgentCompleted,
    AgentFailed,
    AgentStopped,
}

/// M12: which configured command a `TestRun` is for — matches the
/// `project.<project_id>.<kind>_command` setting key's `<kind>` part
/// (`project_detect::project_setting_key`) one-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestRunKind {
    Test,
    Lint,
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestRunStatus {
    Running,
    Success,
    Failure,
}

/// M12: one manually-triggered run of a project's configured test/lint/build
/// command (`commands::testing_commands::run_test_suite`), independent of
/// any agent run — the Testing tab's history list is built from these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRun {
    pub id: String,
    pub project_id: String,
    pub kind: TestRunKind,
    pub command: String,
    pub status: TestRunStatus,
    pub output: Option<String>,
    pub exit_code: Option<i64>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

/// M14: the reviewer agent's own review categories — matches the Phase 4
/// plan's own category list exactly. Stored (lowercased via
/// `rename_all = "snake_case"`) as the `category` field inside a `Review`'s
/// `findings_json`, and as the `submit_review` tool's `category` enum
/// (`agent::reviewer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCategory {
    Correctness,
    Security,
    Performance,
    Maintainability,
    Tests,
    Architecture,
    Style,
}

/// M14: how serious one finding is — drives both display (badges) and
/// `agent::reviewer::should_create_follow_up`'s decision to file a real
/// follow-up task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// M14: one issue the reviewer found. `file`/`line` are best-effort —
/// the model may not always pin a finding to an exact location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    pub category: ReviewCategory,
    pub severity: ReviewSeverity,
    pub summary: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
}

/// M14: a `reviews` row's lifecycle — see `migrations/0008_reviews.sql`'s
/// own docs for exactly when each value applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    Passed,
    Failed,
}

/// M14: one reviewer run's persisted result for a completed `agent_runs`
/// row. `findings_json` is the serialized `Vec<ReviewFinding>` — kept as raw
/// text at the DB layer (one column, no separate findings table) and parsed
/// back into a real `Vec<ReviewFinding>` for the `ReviewDto` the frontend
/// actually consumes (`commands::review_commands`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub id: String,
    pub agent_run_id: String,
    pub score: i64,
    pub findings_json: String,
    pub status: ReviewStatus,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    pub project_id: Option<String>,
    pub agent_run_id: Option<String>,
    #[serde(rename = "type")]
    pub notification_type: NotificationType,
    pub title: String,
    pub body: Option<String>,
    pub is_read: bool,
    pub created_at: String,
}

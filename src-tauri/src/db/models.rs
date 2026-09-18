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
}

/// A mission (M8): a plain-English objective, turned into a structured task
/// plan by one Anthropic Messages API call (`orchestrator::planner`), which
/// the user reviews and approves before anything executes. Execution of an
/// `approved` mission is future work (M9) — nothing in this milestone
/// advances a mission past `approved`.
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

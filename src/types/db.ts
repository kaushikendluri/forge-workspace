/**
 * TypeScript mirrors of the SQLite schema defined in
 * `src-tauri/migrations/0001_init.sql`.
 *
 * These types describe the shape of rows as they will come back from Tauri
 * commands starting in M2+. In M1 nothing populates them — stores hold
 * empty/null state — but the shapes are defined now so route/store code can
 * be written against the real contract.
 */

export type IsoDateTime = string;

export interface Project {
  id: string;
  name: string;
  description: string | null;
  createdAt: IsoDateTime;
  updatedAt: IsoDateTime;
  lastOpenedAt: IsoDateTime | null;
}

export type VcsType = "git";

export interface Repository {
  id: string;
  projectId: string;
  rootPath: string;
  remoteUrl: string | null;
  defaultBranch: string;
  vcsType: VcsType;
  createdAt: IsoDateTime;
}

/**
 * A project row joined with its (Phase 1: single) repository — the shape
 * returned by the `open_project`/`init_project`/`list_projects` Tauri
 * commands and rendered by the Dashboard/Projects routes. Mirrors
 * `src-tauri/src/commands/project_commands.rs`'s `ProjectDto`.
 */
export interface ProjectDto extends Project {
  repositoryId: string;
  rootPath: string;
  remoteUrl: string | null;
  defaultBranch: string;
  vcsType: VcsType;
}

/** One tracked file's status on one side (index or worktree) of a change. */
export interface FileStatusEntry {
  path: string;
  statusCode: string;
}

/**
 * Working-tree status for a repository, from `git status --porcelain=v2`.
 * Mirrors `src-tauri/src/git/mod.rs`'s `GitStatus`.
 */
export interface GitStatus {
  currentBranch: string | null;
  staged: FileStatusEntry[];
  unstaged: FileStatusEntry[];
  untracked: string[];
}

/** Mirrors `src-tauri/src/git/mod.rs`'s `BranchInfo`. */
export interface BranchInfo {
  name: string;
  isCurrent: boolean;
}

/**
 * Full before/after file text for the Changes tab's diff viewer. Mirrors
 * `src-tauri/src/git/mod.rs`'s `GitFileDiff` — `original`/`modified` are
 * `""` for a new/deleted file respectively, rather than the command
 * erroring.
 */
export interface GitFileDiff {
  original: string;
  modified: string;
  isNewFile: boolean;
  isDeleted: boolean;
}

/** One `git log` entry. Mirrors `src-tauri/src/git/mod.rs`'s `CommitInfo`. */
export interface CommitInfo {
  sha: string;
  shortSha: string;
  author: string;
  email: string;
  /** ISO 8601 author date. */
  date: string;
  subject: string;
}

/** One directory entry. Mirrors `src-tauri/src/commands/fs_commands.rs`'s `DirEntryDto`. */
export interface DirEntryDto {
  name: string;
  path: string;
  isDir: boolean;
  /** `null` for directories. */
  sizeBytes: number | null;
}

/**
 * A (possibly truncated) file preview. Mirrors
 * `src-tauri/src/commands/fs_commands.rs`'s `FilePreviewDto`.
 */
export interface FilePreviewDto {
  /** Empty when `isBinary` is true. */
  content: string;
  truncated: boolean;
  isBinary: boolean;
}

export type AgentStatus = "idle" | "running" | "completed" | "failed" | "stopped";

export interface Agent {
  id: string;
  projectId: string;
  repositoryId: string;
  name: string;
  status: AgentStatus;
  systemPrompt: string | null;
  createdAt: IsoDateTime;
  updatedAt: IsoDateTime;
}

export type AgentRunStatus = "queued" | "running" | "completed" | "failed" | "stopped";
export type AgentRunStopReason =
  | "completed"
  | "max_iterations"
  | "user_stopped"
  | "error"
  | "test_fix_budget_exhausted";

export interface AgentRun {
  id: string;
  agentId: string;
  workspaceId: string | null;
  taskPrompt: string;
  modelId: string;
  status: AgentRunStatus;
  stopReason: AgentRunStopReason | null;
  errorMessage: string | null;
  iterationCount: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  /** M13: the self-healing test-fix cycle's bounded retry counter so far. */
  testFixAttempts: number;
  startedAt: IsoDateTime | null;
  completedAt: IsoDateTime | null;
}

export type WorkspaceKind = "primary" | "agent";
export type WorkspaceStatus = "active" | "removed";

export interface Workspace {
  id: string;
  repositoryId: string;
  agentRunId: string | null;
  kind: WorkspaceKind;
  path: string;
  branchName: string;
  baseBranch: string | null;
  baseCommitSha: string | null;
  status: WorkspaceStatus;
  createdAt: IsoDateTime;
  removedAt: IsoDateTime | null;
}

/**
 * One changed file in an agent run's workspace, for `AgentDetail.tsx`'s
 * diff view. Mirrors `src-tauri/src/commands/agent_run_commands.rs`'s
 * `AgentRunFileDiffDto` — a `GitFileDiff` (see `GitFileDiff` above) with the
 * path that produced it attached, since a run's diff is a whole list of
 * files rather than one selected file.
 */
export interface AgentRunFileDiffDto {
  path: string;
  original: string;
  modified: string;
  isNewFile: boolean;
  isDeleted: boolean;
}

export type ToolCallStatus = "running" | "success" | "error";

export interface ToolCall {
  id: string;
  agentRunId: string;
  sequenceNumber: number;
  toolUseId: string;
  toolName: string;
  inputJson: string;
  outputJson: string | null;
  status: ToolCallStatus;
  errorMessage: string | null;
  startedAt: IsoDateTime;
  completedAt: IsoDateTime | null;
  durationMs: number | null;
}

export type ActivityEventType =
  | "run_started"
  | "model_message"
  | "tool_call_started"
  | "tool_call_completed"
  | "run_completed"
  | "run_stopped"
  | "error"
  | "test_fix_cycle";

export interface ActivityEvent {
  id: string;
  agentRunId: string | null;
  toolCallId: string | null;
  eventType: ActivityEventType;
  payloadJson: string;
  createdAt: IsoDateTime;
}

/**
 * `backlog` is a task proposed by a mission's plan (M8) that hasn't been
 * started yet — distinct from `todo` so a freshly-approved plan's tasks read
 * honestly rather than looking like manually triaged work. M9 adds the
 * terminal states the scheduler (`orchestrator::scheduler`) produces once a
 * mission actually runs: `failed` (its own agent run didn't succeed),
 * `blocked` (its dependency ended failed/blocked/cancelled, or it's part of
 * a dependency cycle), `cancelled` (the mission was stopped before this task
 * got a chance to run).
 */
export type TaskStatus = "backlog" | "todo" | "in_progress" | "done" | "failed" | "blocked" | "cancelled";
export type TaskPriority = "low" | "medium" | "high";

export interface Task {
  id: string;
  projectId: string;
  /** The mission (M8) that proposed this task, if any — `null` for a task created directly. */
  missionId: string | null;
  title: string;
  description: string | null;
  status: TaskStatus;
  priority: TaskPriority;
  /** Display order within its mission's plan (0-based). */
  position: number;
  /** Another task that must complete before this one can start, if any. */
  dependsOnTaskId: string | null;
  /**
   * The plan's suggested specialist role for this task (e.g. "frontend",
   * "backend", "database", "qa") — a free-form label, not a foreign key to
   * anything (Agent Skills don't exist until Phase 5).
   */
  agentType: string | null;
  agentRunId: string | null;
  createdAt: IsoDateTime;
  updatedAt: IsoDateTime;
}

/**
 * M11: one column of `Tasks.tsx`'s Kanban board. Mirrors
 * `src-tauri/src/orchestrator/scheduler.rs`'s `BoardColumn` exactly —
 * `column` on `TaskBoardEntryDto` is computed backend-side (from the same
 * dependency-graph logic the scheduler itself uses to decide what to run
 * next), never re-derived in the frontend. M14: `"review"` is now genuinely
 * populated — a `done` task whose agent run has a currently-`pending`
 * `reviews` row (`agent::reviewer`, auto-triggered by the scheduler once a
 * task completes) sits here until that review finishes.
 */
export type BoardColumn = "backlog" | "ready" | "running" | "blocked" | "review" | "complete" | "failed" | "cancelled";

/**
 * One row of the Kanban board: every `Task` field (flattened) plus its
 * derived `column`. Mirrors `src-tauri/src/commands/mission_commands.rs`'s
 * `TaskBoardEntryDto`, returned by `list_mission_board`. M14:
 * `reviewScore`/`reviewStatus` mirror the task's agent run's latest review
 * (if any has ever completed), from the same batched lookup that decides
 * `column` — `null` when no review was ever requested for this task.
 */
export interface TaskBoardEntryDto extends Task {
  column: BoardColumn;
  reviewScore: number | null;
  reviewStatus: ReviewStatus | null;
}

/**
 * M14: the reviewer agent's fixed review categories. Mirrors
 * `src-tauri/src/db/models.rs`'s `ReviewCategory`.
 */
export type ReviewCategory = "correctness" | "security" | "performance" | "maintainability" | "tests" | "architecture" | "style";

/** M14: how serious one finding is. Mirrors `src-tauri/src/db/models.rs`'s `ReviewSeverity`. */
export type ReviewSeverity = "low" | "medium" | "high" | "critical";

/**
 * M14: one issue the reviewer found. `file`/`line` are best-effort — the
 * model may not always pin a finding to an exact location. Mirrors
 * `src-tauri/src/db/models.rs`'s `ReviewFinding`.
 */
export interface ReviewFinding {
  category: ReviewCategory;
  severity: ReviewSeverity;
  summary: string;
  file: string | null;
  line: number | null;
}

/**
 * M14: a review's lifecycle — `pending` while the reviewer is still
 * gathering context/producing its verdict, then `passed`/`failed` based on
 * `agent::reviewer::PASS_THRESHOLD` (70/100). Mirrors
 * `src-tauri/src/db/models.rs`'s `ReviewStatus`.
 */
export type ReviewStatus = "pending" | "passed" | "failed";

/**
 * M14: a completed reviewer pass for an agent run — score, structured
 * findings, and pass/fail status. Mirrors
 * `src-tauri/src/commands/review_commands.rs`'s `ReviewDto`, returned by
 * `request_review`/`get_review`.
 */
export interface ReviewDto {
  id: string;
  agentRunId: string;
  score: number;
  findings: ReviewFinding[];
  status: ReviewStatus;
  createdAt: IsoDateTime;
}

/**
 * M14: the raw `reviews` row shape, with `findingsJson` still a serialized
 * string rather than parsed — this is what `review:updated`'s event payload
 * actually carries (mirrors `src-tauri/src/db/models.rs`'s `Review`
 * verbatim), unlike `ReviewDto` (the command-layer shape with `findings`
 * already parsed). `src/lib/events.ts`/consumers parse `findingsJson`
 * themselves, the same way `commands::review_commands::to_dto` does
 * backend-side.
 */
export interface Review {
  id: string;
  agentRunId: string;
  score: number;
  findingsJson: string;
  status: ReviewStatus;
  createdAt: IsoDateTime;
}

export type MissionStatus = "planning" | "plan_ready" | "approved" | "running" | "completed" | "failed" | "stopped";

/**
 * A mission (M8): a plain-English objective, turned into a structured task
 * plan (real `Task` rows with `missionId` set) by one Anthropic call, which
 * the user reviews and approves before anything executes. Mirrors
 * `src-tauri/src/db/models.rs`'s `Mission`. M9's `orchestrator::scheduler`
 * walks an `approved` mission's task graph and runs each task through the
 * real M5/M6 agent pipeline, moving it to `running`, then
 * `completed`/`failed`/`stopped`.
 */
export interface Mission {
  id: string;
  projectId: string;
  objective: string;
  status: MissionStatus;
  /** The raw `propose_plan` model output, once the plan is ready. */
  planJson: string | null;
  /** The real planner error when `status` is `failed` — never a generic message. */
  errorMessage: string | null;
  createdAt: IsoDateTime;
  approvedAt: IsoDateTime | null;
  completedAt: IsoDateTime | null;
}

export interface ModelConfig {
  id: string;
  provider: string;
  modelId: string;
  displayName: string;
  isDefault: boolean;
  maxOutputTokens: number;
  createdAt: IsoDateTime;
}

export interface Setting {
  key: string;
  value: string;
  updatedAt: IsoDateTime;
}

export type NotificationType = "agent_completed" | "agent_failed" | "agent_stopped";

export interface Notification {
  id: string;
  projectId: string | null;
  agentRunId: string | null;
  type: NotificationType;
  title: string;
  body: string | null;
  isRead: boolean;
  createdAt: IsoDateTime;
}

/**
 * M11: one agent-to-agent structured message within a mission, written by
 * the `send_message` tool available to a mission-context agent run (see
 * `src-tauri/src/agent/tools.rs`) and read back by `list_agent_messages` for
 * Mission Control's messages panel. Mirrors
 * `src-tauri/src/db/models.rs`'s `AgentMessage`. `toAgentRunId` is `null`
 * for a mission-wide broadcast (no specific recipient task was named, or
 * that task hadn't run yet when the message was sent) — this is a
 * persisted log, not a live chat, so it may also point at a run that has
 * since finished.
 */
export interface AgentMessage {
  id: string;
  fromAgentRunId: string;
  toAgentRunId: string | null;
  missionId: string;
  subject: string;
  body: string;
  createdAt: IsoDateTime;
}

/**
 * M12: which configured command a `TestRun` is for — matches the
 * `project.<projectId>.<kind>_command` setting key's `<kind>` part.
 */
export type TestRunKind = "test" | "lint" | "build";
export type TestRunStatus = "running" | "success" | "failure";

/**
 * M12: one manually-triggered run of a project's configured test/lint/build
 * command, independent of any agent run. Mirrors
 * `src-tauri/src/db/models.rs`'s `TestRun`. Returned by `run_test_suite` and
 * `list_test_runs` — the Testing tab's history is built from these.
 */
export interface TestRun {
  id: string;
  projectId: string;
  kind: TestRunKind;
  command: string;
  status: TestRunStatus;
  output: string | null;
  exitCode: number | null;
  startedAt: IsoDateTime;
  completedAt: IsoDateTime | null;
}

/**
 * M12: one configured command's value plus its provenance — `"detected"`
 * (pre-filled by `project_detect::detect_commands` when the project was
 * opened/created and nothing was configured yet), `"user"` (typed into the
 * Testing tab), or `null` (never configured, or configured before M12 with
 * no recorded source). Mirrors
 * `src-tauri/src/commands/testing_commands.rs`'s `CommandSettingDto`.
 */
export interface CommandSettingDto {
  value: string | null;
  source: "detected" | "user" | null;
}

/** Mirrors `src-tauri/src/commands/testing_commands.rs`'s `ProjectCommandSettingsDto`. */
export interface ProjectCommandSettingsDto {
  testCommand: CommandSettingDto;
  lintCommand: CommandSettingDto;
  buildCommand: CommandSettingDto;
}

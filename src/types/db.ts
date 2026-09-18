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
export type AgentRunStopReason = "completed" | "max_iterations" | "user_stopped" | "error";

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
  | "error";

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
 * honestly rather than looking like manually triaged work.
 */
export type TaskStatus = "backlog" | "todo" | "in_progress" | "done";
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

export type MissionStatus = "planning" | "plan_ready" | "approved" | "running" | "completed" | "failed";

/**
 * A mission (M8): a plain-English objective, turned into a structured task
 * plan (real `Task` rows with `missionId` set) by one Anthropic call, which
 * the user reviews and approves before anything executes. Mirrors
 * `src-tauri/src/db/models.rs`'s `Mission`. Nothing in this phase advances a
 * mission past `approved` — execution is a later milestone.
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

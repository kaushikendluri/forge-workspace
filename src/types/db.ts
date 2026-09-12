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

export type TaskStatus = "todo" | "in_progress" | "done";

export interface Task {
  id: string;
  projectId: string;
  title: string;
  description: string | null;
  status: TaskStatus;
  agentRunId: string | null;
  createdAt: IsoDateTime;
  updatedAt: IsoDateTime;
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

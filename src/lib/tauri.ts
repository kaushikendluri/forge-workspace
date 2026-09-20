/**
 * Thin, typed wrapper around `@tauri-apps/api`'s `invoke`.
 *
 * Each command name below corresponds 1:1 to a `#[tauri::command]` function
 * registered in `src-tauri/src/lib.rs` (see `src-tauri/src/commands/*.rs`).
 * Tauri v2 converts each Rust `snake_case` parameter name to `camelCase` for
 * the JS-side invoke args object, so e.g. Rust's `repo_path: String` is
 * called here as `{ repoPath }`.
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  ActivityEvent,
  Agent,
  AgentMessage,
  AgentRun,
  AgentRunFileDiffDto,
  BranchInfo,
  CommitInfo,
  DirEntryDto,
  FilePreviewDto,
  GitFileDiff,
  GitStatus,
  Mission,
  ModelConfig,
  Notification,
  Project,
  ProjectCommandSettingsDto,
  ProjectDto,
  Repository,
  Task,
  TaskBoardEntryDto,
  TestRun,
  TestRunKind,
  ToolCall,
  Workspace,
} from "@/types/db";

// TODO(M5+): fill in remaining request/response types (workspaces,
// agents/tasks) as each command lands; keep it centralized so `invokeCommand`
// stays the only place that calls `@tauri-apps/api`'s `invoke`.
export interface CommandMap {
  open_project: { args: { path: string }; result: ProjectDto };
  init_project: { args: { path: string; name: string }; result: ProjectDto };
  list_projects: { args: Record<string, never>; result: ProjectDto[] };
  get_project: { args: { projectId: string }; result: Project | null };
  create_project: { args: { name: string; description?: string }; result: Project };
  git_status: { args: { repoPath: string }; result: GitStatus };
  git_branches: { args: { repoPath: string }; result: BranchInfo[] };
  git_current_branch: { args: { repoPath: string }; result: string | null };
  git_diff_file: { args: { repoPath: string; filePath: string }; result: GitFileDiff };
  git_log: { args: { repoPath: string; limit: number }; result: CommitInfo[] };
  list_directory: { args: { dirPath: string }; result: DirEntryDto[] };
  read_file_preview: { args: { filePath: string; maxBytes: number }; result: FilePreviewDto };
  terminal_spawn: { args: { cwd: string }; result: string };
  terminal_write: { args: { terminalId: string; data: string }; result: void };
  terminal_resize: { args: { terminalId: string; cols: number; rows: number }; result: void };
  terminal_kill: { args: { terminalId: string }; result: void };
  open_repository: { args: { rootPath: string }; result: Repository };
  list_workspaces: { args: { repositoryId: string }; result: Workspace[] };
  list_tasks: { args: { projectId: string }; result: Task[] };
  list_agents: { args: { projectId: string }; result: Agent[] };
  create_agent: { args: { projectId: string; name: string }; result: Agent };
  start_worktree_for_agent: { args: { agentId: string; taskPrompt: string }; result: Workspace };
  remove_agent_workspace: { args: { workspaceId: string }; result: void };
  start_agent_run: { args: { agentRunId: string }; result: void };
  stop_agent_run: { args: { agentRunId: string }; result: void };
  get_agent_run: { args: { agentRunId: string }; result: AgentRun };
  list_agent_runs: { args: { agentId: string }; result: AgentRun[] };
  list_tool_calls: { args: { agentRunId: string }; result: ToolCall[] };
  list_activity_events: { args: { agentRunId: string }; result: ActivityEvent[] };
  get_run_diff: { args: { agentRunId: string }; result: AgentRunFileDiffDto[] };
  get_setting: { args: { key: string }; result: string | null };
  set_setting: { args: { key: string; value: string }; result: void };
  list_settings: { args: Record<string, never>; result: Record<string, string> };
  set_api_key: { args: { key: string }; result: void };
  has_api_key: { args: Record<string, never>; result: boolean };
  clear_api_key: { args: Record<string, never>; result: void };
  list_model_configs: { args: Record<string, never>; result: ModelConfig[] };
  list_notifications: { args: { projectId: string | null }; result: Notification[] };
  mark_notification_read: { args: { id: string }; result: void };
  unread_notification_count: { args: { projectId: string | null }; result: number };
  create_mission: { args: { projectId: string; objective: string }; result: Mission };
  approve_mission_plan: { args: { missionId: string }; result: void };
  get_mission: { args: { missionId: string }; result: Mission };
  list_missions: { args: { projectId: string }; result: Mission[] };
  list_mission_tasks: { args: { missionId: string }; result: Task[] };
  list_mission_board: { args: { missionId: string }; result: TaskBoardEntryDto[] };
  list_agent_messages: { args: { missionId: string }; result: AgentMessage[] };
  start_mission: { args: { missionId: string }; result: void };
  stop_mission: { args: { missionId: string }; result: void };
  get_project_command_settings: { args: { projectId: string }; result: ProjectCommandSettingsDto };
  set_project_command_setting: { args: { projectId: string; kind: TestRunKind; value: string }; result: void };
  run_test_suite: { args: { projectId: string; kind: TestRunKind }; result: TestRun };
  list_test_runs: { args: { projectId: string; kind: TestRunKind | null }; result: TestRun[] };
}

/**
 * Typed `invoke` — call as `invokeCommand("list_projects", {})`.
 */
export async function invokeCommand<K extends keyof CommandMap>(
  command: K,
  args: CommandMap[K]["args"],
): Promise<CommandMap[K]["result"]> {
  return invoke<CommandMap[K]["result"]>(command, args);
}

/** True when running inside the Tauri webview (vs. a plain browser dev preview). */
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Registers (or re-opens) the git repository at `path` as a project. */
export async function openProject(path: string): Promise<ProjectDto> {
  return invokeCommand("open_project", { path });
}

/** Creates `path` if missing, `git init`s it, then registers it as `name`. */
export async function initProject(path: string, name: string): Promise<ProjectDto> {
  return invokeCommand("init_project", { path, name });
}

/** All known projects, most recently updated first. */
export async function listProjects(): Promise<ProjectDto[]> {
  return invokeCommand("list_projects", {});
}

/** Working-tree status (branch + staged/unstaged/untracked files). */
export async function gitStatus(repoPath: string): Promise<GitStatus> {
  return invokeCommand("git_status", { repoPath });
}

/** All local branches, with the checked-out one flagged. */
export async function gitBranches(repoPath: string): Promise<BranchInfo[]> {
  return invokeCommand("git_branches", { repoPath });
}

/** The currently checked-out branch, or `null` if HEAD is detached. */
export async function gitCurrentBranch(repoPath: string): Promise<string | null> {
  return invokeCommand("git_current_branch", { repoPath });
}

/**
 * Full before/after text for `filePath` (relative to `repoPath`), for the
 * Changes tab's diff viewer. Handles new/deleted files rather than erroring.
 */
export async function gitDiffFile(repoPath: string, filePath: string): Promise<GitFileDiff> {
  return invokeCommand("git_diff_file", { repoPath, filePath });
}

/** The `limit` most recent commits for the repository at `repoPath`. */
export async function gitLog(repoPath: string, limit: number): Promise<CommitInfo[]> {
  return invokeCommand("git_log", { repoPath, limit });
}

/** Lists the contents of `dirPath` (directories first, then alphabetical). */
export async function listDirectory(dirPath: string): Promise<DirEntryDto[]> {
  return invokeCommand("list_directory", { dirPath });
}

/** Reads up to `maxBytes` of `filePath`, reporting truncation/binary-ness. */
export async function readFilePreview(filePath: string, maxBytes: number): Promise<FilePreviewDto> {
  return invokeCommand("read_file_preview", { filePath, maxBytes });
}

/** Spawns a new PTY-backed shell session rooted at `cwd`; returns its id. */
export async function terminalSpawn(cwd: string): Promise<string> {
  return invokeCommand("terminal_spawn", { cwd });
}

/** Writes raw keystrokes to the terminal session's pty stdin. */
export async function terminalWrite(terminalId: string, data: string): Promise<void> {
  return invokeCommand("terminal_write", { terminalId, data });
}

/** Resizes the terminal session's pty to match the frontend's dimensions. */
export async function terminalResize(terminalId: string, cols: number, rows: number): Promise<void> {
  return invokeCommand("terminal_resize", { terminalId, cols, rows });
}

/** Kills the terminal session's child process. */
export async function terminalKill(terminalId: string): Promise<void> {
  return invokeCommand("terminal_kill", { terminalId });
}

/** The stored value for `key`, or `null` if it's never been set. */
export async function getSetting(key: string): Promise<string | null> {
  return invokeCommand("get_setting", { key });
}

/** Inserts or updates the value for `key`. */
export async function setSetting(key: string, value: string): Promise<void> {
  return invokeCommand("set_setting", { key, value });
}

/** All settings currently stored, as a flat `key -> value` map. */
export async function listSettings(): Promise<Record<string, string>> {
  return invokeCommand("list_settings", {});
}

/** Stores the Anthropic API key in the OS keychain (never SQLite). */
export async function setApiKey(key: string): Promise<void> {
  return invokeCommand("set_api_key", { key });
}

/** Whether an Anthropic API key is currently stored. Never returns the key itself. */
export async function hasApiKey(): Promise<boolean> {
  return invokeCommand("has_api_key", {});
}

/** Removes the stored Anthropic API key, if any. */
export async function clearApiKey(): Promise<void> {
  return invokeCommand("clear_api_key", {});
}

/** The known model configs (seeded with one Claude Sonnet 5 default), default first. */
export async function listModelConfigs(): Promise<ModelConfig[]> {
  return invokeCommand("list_model_configs", {});
}

/** Notifications for `projectId` (or every project, if `null`), most recent first. */
export async function listNotifications(projectId: string | null): Promise<Notification[]> {
  return invokeCommand("list_notifications", { projectId });
}

/** Marks a single notification as read. */
export async function markNotificationRead(id: string): Promise<void> {
  return invokeCommand("mark_notification_read", { id });
}

/** Count of unread notifications for `projectId` (or every project, if `null`). */
export async function unreadNotificationCount(projectId: string | null): Promise<number> {
  return invokeCommand("unread_notification_count", { projectId });
}

/**
 * Creates a mission for `projectId` and runs the real mission planner for
 * `objective` end to end (one Anthropic call — this can take several
 * seconds). Always resolves with the mission row, whether planning
 * succeeded (`status: "plan_ready"`, real `Task` rows created) or failed
 * (`status: "failed"`, `errorMessage` set to the real error) — it never
 * rejects for a planner failure, only for a structural problem (e.g. an
 * unknown project).
 */
export async function createMission(projectId: string, objective: string): Promise<Mission> {
  return invokeCommand("create_mission", { projectId, objective });
}

/** Records human approval of a `plan_ready` mission's plan. Starts nothing — execution is a future milestone. */
export async function approveMissionPlan(missionId: string): Promise<void> {
  return invokeCommand("approve_mission_plan", { missionId });
}

/** The current state of one mission. */
export async function getMission(missionId: string): Promise<Mission> {
  return invokeCommand("get_mission", { missionId });
}

/** All missions for `projectId`, most recently created first. */
export async function listMissions(projectId: string): Promise<Mission[]> {
  return invokeCommand("list_missions", { projectId });
}

/** The tasks `missionId`'s plan proposed, in plan order. */
export async function listMissionTasks(missionId: string): Promise<Task[]> {
  return invokeCommand("list_mission_tasks", { missionId });
}

/**
 * M11: `missionId`'s tasks, each labeled with its derived Kanban board
 * column (backend-computed from the scheduler's own dependency-graph
 * readiness logic — see `orchestrator::scheduler::compute_board_columns`).
 * The board `Tasks.tsx` renders is built from this, not `listMissionTasks`.
 */
export async function listMissionBoard(missionId: string): Promise<TaskBoardEntryDto[]> {
  return invokeCommand("list_mission_board", { missionId });
}

/** M11: the full agent-to-agent message log for `missionId`, oldest first. */
export async function listAgentMessages(missionId: string): Promise<AgentMessage[]> {
  return invokeCommand("list_agent_messages", { missionId });
}

/**
 * Starts real execution of an `approved` mission's plan
 * (`orchestrator::scheduler::run_mission`): walks the task dependency
 * graph and runs each task, sequentially, through the real M5/M6 agent
 * pipeline. Progress arrives entirely through `mission:*` events — this
 * only kicks the run off. Rejects if `missionId` isn't currently `approved`.
 */
export async function startMission(missionId: string): Promise<void> {
  return invokeCommand("start_mission", { missionId });
}

/** Cancels a currently-running mission. Errors if `missionId` isn't active. */
export async function stopMission(missionId: string): Promise<void> {
  return invokeCommand("stop_mission", { missionId });
}

/** All agents for `projectId`, most recently created first. */
export async function listAgents(projectId: string): Promise<Agent[]> {
  return invokeCommand("list_agents", { projectId });
}

/** Creates a new agent (status `idle`) for `projectId`'s repository. */
export async function createAgent(projectId: string, name: string): Promise<Agent> {
  return invokeCommand("create_agent", { projectId, name });
}

/**
 * The M5-scoped "Start" action: creates a real git worktree + branch for
 * `agentId` and records a queued run for `taskPrompt`. No model call is
 * made — agent execution lands in a later milestone.
 */
export async function startWorktreeForAgent(agentId: string, taskPrompt: string): Promise<Workspace> {
  return invokeCommand("start_worktree_for_agent", { agentId, taskPrompt });
}

/** Removes the worktree backing `workspaceId` and marks it removed. Fails if the worktree has uncommitted changes. */
export async function removeAgentWorkspace(workspaceId: string): Promise<void> {
  return invokeCommand("remove_agent_workspace", { workspaceId });
}

/**
 * Starts the real tool-calling loop for `agentRunId`, which must currently
 * be `queued` (i.e. `startWorktreeForAgent` already ran for it). Progress
 * arrives entirely through events (`agent-run:*`) — this only kicks the run
 * off.
 */
export async function startAgentRun(agentRunId: string): Promise<void> {
  return invokeCommand("start_agent_run", { agentRunId });
}

/** Cancels a currently-active run. Errors if `agentRunId` isn't active. */
export async function stopAgentRun(agentRunId: string): Promise<void> {
  return invokeCommand("stop_agent_run", { agentRunId });
}

/** The current state of one agent run. */
export async function getAgentRun(agentRunId: string): Promise<AgentRun> {
  return invokeCommand("get_agent_run", { agentRunId });
}

/** All runs for `agentId`, most recently started first. */
export async function listAgentRuns(agentId: string): Promise<AgentRun[]> {
  return invokeCommand("list_agent_runs", { agentId });
}

/** All tool calls for a run, in call order. */
export async function listToolCalls(agentRunId: string): Promise<ToolCall[]> {
  return invokeCommand("list_tool_calls", { agentRunId });
}

/** The full activity feed for a run, oldest first. */
export async function listActivityEvents(agentRunId: string): Promise<ActivityEvent[]> {
  return invokeCommand("list_activity_events", { agentRunId });
}

/** A real diff of everything currently changed in the run's workspace. */
export async function getRunDiff(agentRunId: string): Promise<AgentRunFileDiffDto[]> {
  return invokeCommand("get_run_diff", { agentRunId });
}

/**
 * M12: `projectId`'s detected/configured test/lint/build commands, each
 * labeled with whether it was auto-detected, user-set, or never configured.
 */
export async function getProjectCommandSettings(projectId: string): Promise<ProjectCommandSettingsDto> {
  return invokeCommand("get_project_command_settings", { projectId });
}

/** Sets `projectId`'s `kind` command and marks it as user-configured. */
export async function setProjectCommandSetting(projectId: string, kind: TestRunKind, value: string): Promise<void> {
  return invokeCommand("set_project_command_setting", { projectId, kind, value });
}

/**
 * Runs `projectId`'s configured `kind` command (in the project's primary
 * repository root, independent of any agent run) and returns the completed
 * result. Rejects with a clear message if no command is configured for
 * `kind` — never guesses one.
 */
export async function runTestSuite(projectId: string, kind: TestRunKind): Promise<TestRun> {
  return invokeCommand("run_test_suite", { projectId, kind });
}

/** `projectId`'s test/lint/build run history, most recent first, optionally narrowed to one `kind`. */
export async function listTestRuns(projectId: string, kind: TestRunKind | null = null): Promise<TestRun[]> {
  return invokeCommand("list_test_runs", { projectId, kind });
}

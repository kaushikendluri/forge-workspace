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
  Agent,
  BranchInfo,
  CommitInfo,
  DirEntryDto,
  FilePreviewDto,
  GitFileDiff,
  GitStatus,
  Project,
  ProjectDto,
  Repository,
  Setting,
  Task,
  Workspace,
} from "@/types/db";

// TODO(M3+): fill in remaining request/response types (settings, workspaces,
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
  get_setting: { args: { key: string }; result: Setting | null };
  set_setting: { args: { key: string; value: string }; result: void };
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

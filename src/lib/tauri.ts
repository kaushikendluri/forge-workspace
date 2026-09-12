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

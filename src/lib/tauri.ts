/**
 * Thin, typed wrapper around `@tauri-apps/api`'s `invoke`.
 *
 * Nothing calls this in M1 — there is no Rust backend running yet — but the
 * shape is fixed here so route/store code written against it in M2+ doesn't
 * need to change call sites, only implementations.
 *
 * Each command name below corresponds 1:1 to a `#[tauri::command]` function
 * that will be registered in `src-tauri/src/lib.rs` (see
 * `src-tauri/src/commands/*.rs`).
 */

import { invoke } from "@tauri-apps/api/core";
import type { Agent, Project, Repository, Setting, Task, Workspace } from "@/types/db";

// TODO(M2): replace this Command map with real request/response types as
// each command lands; keep it centralized so `invokeCommand` stays the only
// place that calls `@tauri-apps/api`'s `invoke`.
export interface CommandMap {
  list_projects: { args: Record<string, never>; result: Project[] };
  get_project: { args: { projectId: string }; result: Project | null };
  create_project: { args: { name: string; description?: string }; result: Project };
  open_repository: { args: { rootPath: string }; result: Repository };
  list_workspaces: { args: { repositoryId: string }; result: Workspace[] };
  list_tasks: { args: { projectId: string }; result: Task[] };
  list_agents: { args: { projectId: string }; result: Agent[] };
  get_setting: { args: { key: string }; result: Setting | null };
  set_setting: { args: { key: string; value: string }; result: void };
}

/**
 * Typed `invoke` — call as `invokeCommand("list_projects", {})`.
 * Unused until a real Tauri backend is running (M2+).
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

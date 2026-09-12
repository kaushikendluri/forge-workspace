import { create } from "zustand";
import { open } from "@tauri-apps/plugin-dialog";
import { join } from "@tauri-apps/api/path";
import type { ProjectDto } from "@/types/db";
import { gitCurrentBranch, initProject, isTauriRuntime, listProjects, openProject } from "@/lib/tauri";

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

interface ProjectState {
  /** All known projects, loaded from the real SQLite-backed `list_projects` command. */
  projects: ProjectDto[];
  /** Currently open project, if any. */
  activeProjectId: string | null;
  /** Real `git_current_branch` result for the active project's repository root. */
  activeBranch: string | null;
  isLoading: boolean;
  /** Last error from a project command (open/init/list), for inline display. */
  error: string | null;
  setProjects: (projects: ProjectDto[]) => void;
  upsertProject: (project: ProjectDto) => void;
  removeProject: (projectId: string) => void;
  setActiveProjectId: (projectId: string | null) => void;
  setLoading: (loading: boolean) => void;
  setError: (error: string | null) => void;
  /** Loads all projects from the backend (`list_projects`). */
  loadProjects: () => Promise<void>;
  /** Re-fetches `activeBranch` for the currently active project, if any. */
  refreshActiveBranch: () => Promise<void>;
  /**
   * Opens the native folder picker, and on a real selection, registers it as
   * a project via the `open_project` command. Returns the opened project on
   * success, or `null` if the user cancelled or the folder wasn't a git repo
   * (in which case `error` is set to the real backend error message).
   */
  openProjectDialog: () => Promise<ProjectDto | null>;
  /**
   * Opens the native folder picker to choose a parent directory, prompts for
   * a new project folder name, then creates + `git init`s + registers it via
   * the `init_project` command.
   */
  createProjectDialog: () => Promise<ProjectDto | null>;
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  projects: [],
  activeProjectId: null,
  activeBranch: null,
  isLoading: false,
  error: null,
  setProjects: (projects) => set({ projects }),
  upsertProject: (project) =>
    set((s) => {
      const idx = s.projects.findIndex((p) => p.id === project.id);
      if (idx === -1) return { projects: [...s.projects, project] };
      const next = s.projects.slice();
      next[idx] = project;
      return { projects: next };
    }),
  removeProject: (projectId) =>
    set((s) => ({ projects: s.projects.filter((p) => p.id !== projectId) })),
  setActiveProjectId: (projectId) => {
    set({ activeProjectId: projectId, activeBranch: null });
    if (projectId) void get().refreshActiveBranch();
  },
  setLoading: (loading) => set({ isLoading: loading }),
  setError: (error) => set({ error }),

  loadProjects: async () => {
    set({ isLoading: true, error: null });
    try {
      const projects = await listProjects();
      set({ projects });
    } catch (err) {
      set({ error: errorMessage(err) });
    } finally {
      set({ isLoading: false });
    }
  },

  refreshActiveBranch: async () => {
    const { projects, activeProjectId } = get();
    const project = projects.find((p) => p.id === activeProjectId);
    if (!project) return;
    try {
      const branch = await gitCurrentBranch(project.rootPath);
      // Bail if the active project changed while this was in flight.
      if (get().activeProjectId === project.id) set({ activeBranch: branch });
    } catch {
      if (get().activeProjectId === project.id) set({ activeBranch: null });
    }
  },

  openProjectDialog: async () => {
    set({ error: null });
    if (!isTauriRuntime()) {
      set({ error: "Opening a repository requires the desktop app runtime." });
      return null;
    }
    const selected = await open({ directory: true, multiple: false, title: "Open a repository" });
    if (!selected || Array.isArray(selected)) return null;

    set({ isLoading: true });
    try {
      const project = await openProject(selected);
      get().upsertProject(project);
      set({ activeProjectId: project.id, activeBranch: null });
      void get().refreshActiveBranch();
      return project;
    } catch (err) {
      set({ error: errorMessage(err) });
      return null;
    } finally {
      set({ isLoading: false });
    }
  },

  createProjectDialog: async () => {
    set({ error: null });
    if (!isTauriRuntime()) {
      set({ error: "Creating a project requires the desktop app runtime." });
      return null;
    }
    const parentDir = await open({ directory: true, multiple: false, title: "Choose a location" });
    if (!parentDir || Array.isArray(parentDir)) return null;

    const name = window.prompt("New project folder name")?.trim();
    if (!name) return null;

    set({ isLoading: true });
    try {
      const path = await join(parentDir, name);
      const project = await initProject(path, name);
      get().upsertProject(project);
      set({ activeProjectId: project.id, activeBranch: null });
      void get().refreshActiveBranch();
      return project;
    } catch (err) {
      set({ error: errorMessage(err) });
      return null;
    } finally {
      set({ isLoading: false });
    }
  },
}));

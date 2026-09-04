import { create } from "zustand";
import type { Repository, Workspace } from "@/types/db";

interface RepositoryState {
  /** Repositories keyed by project id. Empty in M1. */
  repositoriesByProjectId: Record<string, Repository[]>;
  /** Worktree-backed workspaces keyed by repository id. Empty in M1. */
  workspacesByRepositoryId: Record<string, Workspace[]>;
  setRepositoriesForProject: (projectId: string, repositories: Repository[]) => void;
  setWorkspacesForRepository: (repositoryId: string, workspaces: Workspace[]) => void;
  clear: () => void;
}

export const useRepositoryStore = create<RepositoryState>((set) => ({
  repositoriesByProjectId: {},
  workspacesByRepositoryId: {},
  setRepositoriesForProject: (projectId, repositories) =>
    set((s) => ({
      repositoriesByProjectId: { ...s.repositoriesByProjectId, [projectId]: repositories },
    })),
  setWorkspacesForRepository: (repositoryId, workspaces) =>
    set((s) => ({
      workspacesByRepositoryId: { ...s.workspacesByRepositoryId, [repositoryId]: workspaces },
    })),
  clear: () => set({ repositoriesByProjectId: {}, workspacesByRepositoryId: {} }),
}));

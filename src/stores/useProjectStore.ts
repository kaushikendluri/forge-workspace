import { create } from "zustand";
import type { Project } from "@/types/db";

interface ProjectState {
  /** All known projects. Empty in M1 — nothing populates this until the M2 backend lands. */
  projects: Project[];
  /** Currently open project, if any. */
  activeProjectId: string | null;
  isLoading: boolean;
  setProjects: (projects: Project[]) => void;
  upsertProject: (project: Project) => void;
  removeProject: (projectId: string) => void;
  setActiveProjectId: (projectId: string | null) => void;
  setLoading: (loading: boolean) => void;
}

export const useProjectStore = create<ProjectState>((set) => ({
  projects: [],
  activeProjectId: null,
  isLoading: false,
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
  setActiveProjectId: (projectId) => set({ activeProjectId: projectId }),
  setLoading: (loading) => set({ isLoading: loading }),
}));

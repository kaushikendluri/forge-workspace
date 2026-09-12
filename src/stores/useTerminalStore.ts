import { create } from "zustand";

export type TerminalConnectionStatus = "idle" | "connecting" | "connected" | "exited";

export interface TerminalSession {
  id: string;
  projectId: string;
  status: TerminalConnectionStatus;
  exitCode: number | null;
}

interface TerminalState {
  /** Terminal sessions keyed by (real, backend) terminal id. */
  sessionsById: Record<string, TerminalSession>;
  /** Ordered terminal ids open for each project — a project can have
   * multiple terminal tabs, each backed by its own real pty session. */
  terminalIdsByProjectId: Record<string, string[]>;
  /** Which terminal tab is active, per project. */
  activeTerminalIdByProjectId: Record<string, string | null>;
  /** Registers a newly spawned session (once `terminal_spawn` resolves with
   * a real id) and makes it the project's active tab if none is set yet. */
  addSession: (session: TerminalSession) => void;
  updateSession: (id: string, patch: Partial<TerminalSession>) => void;
  /** Removes a session (once its `TerminalView` unmounts / it's killed),
   * reassigning the project's active tab if it was the one removed. */
  removeSession: (id: string) => void;
  setActiveTerminalId: (projectId: string, id: string | null) => void;
}

export const useTerminalStore = create<TerminalState>((set) => ({
  sessionsById: {},
  terminalIdsByProjectId: {},
  activeTerminalIdByProjectId: {},

  addSession: (session) =>
    set((s) => {
      const existingIds = s.terminalIdsByProjectId[session.projectId] ?? [];
      const ids = existingIds.includes(session.id) ? existingIds : [...existingIds, session.id];
      return {
        sessionsById: { ...s.sessionsById, [session.id]: session },
        terminalIdsByProjectId: { ...s.terminalIdsByProjectId, [session.projectId]: ids },
        activeTerminalIdByProjectId: {
          ...s.activeTerminalIdByProjectId,
          [session.projectId]: s.activeTerminalIdByProjectId[session.projectId] ?? session.id,
        },
      };
    }),

  updateSession: (id, patch) =>
    set((s) => {
      const existing = s.sessionsById[id];
      if (!existing) return s;
      return { sessionsById: { ...s.sessionsById, [id]: { ...existing, ...patch } } };
    }),

  removeSession: (id) =>
    set((s) => {
      const existing = s.sessionsById[id];
      const nextSessions = { ...s.sessionsById };
      delete nextSessions[id];
      if (!existing) return { sessionsById: nextSessions };

      const remainingIds = (s.terminalIdsByProjectId[existing.projectId] ?? []).filter((tid) => tid !== id);
      const wasActive = s.activeTerminalIdByProjectId[existing.projectId] === id;
      const nextActive = wasActive
        ? (remainingIds[remainingIds.length - 1] ?? null)
        : (s.activeTerminalIdByProjectId[existing.projectId] ?? null);

      return {
        sessionsById: nextSessions,
        terminalIdsByProjectId: { ...s.terminalIdsByProjectId, [existing.projectId]: remainingIds },
        activeTerminalIdByProjectId: { ...s.activeTerminalIdByProjectId, [existing.projectId]: nextActive },
      };
    }),

  setActiveTerminalId: (projectId, id) =>
    set((s) => ({
      activeTerminalIdByProjectId: { ...s.activeTerminalIdByProjectId, [projectId]: id },
    })),
}));

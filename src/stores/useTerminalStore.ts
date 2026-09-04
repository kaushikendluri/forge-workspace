import { create } from "zustand";

export type TerminalConnectionStatus = "idle" | "connecting" | "connected" | "exited";

export interface TerminalSession {
  id: string;
  projectId: string;
  status: TerminalConnectionStatus;
  exitCode: number | null;
}

interface TerminalState {
  /** Terminal sessions keyed by id. Empty in M1 — no PTY backend exists yet. */
  sessionsById: Record<string, TerminalSession>;
  activeTerminalId: string | null;
  addSession: (session: TerminalSession) => void;
  updateSession: (id: string, patch: Partial<TerminalSession>) => void;
  removeSession: (id: string) => void;
  setActiveTerminalId: (id: string | null) => void;
}

export const useTerminalStore = create<TerminalState>((set) => ({
  sessionsById: {},
  activeTerminalId: null,
  addSession: (session) =>
    set((s) => ({ sessionsById: { ...s.sessionsById, [session.id]: session } })),
  updateSession: (id, patch) =>
    set((s) => {
      const existing = s.sessionsById[id];
      if (!existing) return s;
      return { sessionsById: { ...s.sessionsById, [id]: { ...existing, ...patch } } };
    }),
  removeSession: (id) =>
    set((s) => {
      const next = { ...s.sessionsById };
      delete next[id];
      return { sessionsById: next };
    }),
  setActiveTerminalId: (id) => set({ activeTerminalId: id }),
}));

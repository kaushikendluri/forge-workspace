import { create } from "zustand";
import type { ActivityEvent, AgentRun, ToolCall } from "@/types/db";

/**
 * Everything `AgentDetail.tsx` renders for one agent run, keyed by
 * `agentRunId`. Mutated only from event listeners (plus one initial load
 * from `list_tool_calls`/`list_activity_events`/`get_agent_run`) — nothing
 * here is ever fabricated client-side.
 */
interface RunViewState {
  run: AgentRun | null;
  /** Oldest first — the order the backend returns/emits them in. */
  activity: ActivityEvent[];
  toolCallsById: Record<string, ToolCall>;
  /** Tool call ids in the order they were first seen (call order). */
  toolCallOrder: string[];
  /** Ephemeral streamed text for the turn currently in flight. Cleared once
   * that turn's `model_message` activity event lands (the persisted text
   * takes over) or a new run is loaded. */
  streamingText: string;
}

function emptyRunState(): RunViewState {
  return { run: null, activity: [], toolCallsById: {}, toolCallOrder: [], streamingText: "" };
}

interface AgentStoreState {
  runsById: Record<string, RunViewState>;
  /** Replaces a run's full state — called once after the initial
   * `get_agent_run`/`list_tool_calls`/`list_activity_events` fetch. */
  loadRun: (agentRunId: string, run: AgentRun, activity: ActivityEvent[], toolCalls: ToolCall[]) => void;
  setRun: (agentRunId: string, run: AgentRun) => void;
  appendActivity: (agentRunId: string, event: ActivityEvent) => void;
  upsertToolCall: (agentRunId: string, toolCall: ToolCall) => void;
  appendMessageDelta: (agentRunId: string, text: string) => void;
}

function getOrCreate(runsById: Record<string, RunViewState>, agentRunId: string): RunViewState {
  return runsById[agentRunId] ?? emptyRunState();
}

export const useAgentStore = create<AgentStoreState>((set) => ({
  runsById: {},

  loadRun: (agentRunId, run, activity, toolCalls) =>
    set((s) => {
      const toolCallsById: Record<string, ToolCall> = {};
      const toolCallOrder: string[] = [];
      for (const call of toolCalls) {
        toolCallsById[call.id] = call;
        toolCallOrder.push(call.id);
      }
      return { runsById: { ...s.runsById, [agentRunId]: { run, activity, toolCallsById, toolCallOrder, streamingText: "" } } };
    }),

  setRun: (agentRunId, run) =>
    set((s) => ({
      runsById: { ...s.runsById, [agentRunId]: { ...getOrCreate(s.runsById, agentRunId), run } },
    })),

  appendActivity: (agentRunId, event) =>
    set((s) => {
      const existing = getOrCreate(s.runsById, agentRunId);
      if (existing.activity.some((e) => e.id === event.id)) return s;
      return {
        runsById: {
          ...s.runsById,
          [agentRunId]: { ...existing, activity: [...existing.activity, event], streamingText: "" },
        },
      };
    }),

  upsertToolCall: (agentRunId, toolCall) =>
    set((s) => {
      const existing = getOrCreate(s.runsById, agentRunId);
      const isNew = !existing.toolCallsById[toolCall.id];
      return {
        runsById: {
          ...s.runsById,
          [agentRunId]: {
            ...existing,
            toolCallsById: { ...existing.toolCallsById, [toolCall.id]: toolCall },
            toolCallOrder: isNew ? [...existing.toolCallOrder, toolCall.id] : existing.toolCallOrder,
          },
        },
      };
    }),

  appendMessageDelta: (agentRunId, text) =>
    set((s) => {
      const existing = getOrCreate(s.runsById, agentRunId);
      return {
        runsById: { ...s.runsById, [agentRunId]: { ...existing, streamingText: existing.streamingText + text } },
      };
    }),
}));

/**
 * TypeScript payload shapes for Tauri events emitted by the Rust backend
 * (see `src-tauri/src/events/mod.rs`). Nothing emits these yet in M1 — the
 * backend doesn't exist — but `src/lib/events.ts` is typed against these
 * contracts so M2+ wiring is a matter of implementation, not design.
 */

import type {
  ActivityEvent,
  AgentRunStatus,
  AgentStatus,
  MissionStatus,
  Notification,
  TaskStatus,
  ToolCall,
} from "./db";

export interface AgentStatusChangedEvent {
  agentId: string;
  status: AgentStatus;
}

export interface AgentRunStatusChangedEvent {
  agentRunId: string;
  agentId: string;
  status: AgentRunStatus;
}

export interface AgentRunActivityEvent {
  agentRunId: string;
  event: ActivityEvent;
}

export interface ToolCallUpdatedEvent {
  agentRunId: string;
  toolCall: ToolCall;
}

/** Ephemeral streamed assistant text — never persisted; a `model_message`
 * `AgentRunActivityEvent` carries the final text once the turn completes. */
export interface AgentRunMessageDeltaEvent {
  agentRunId: string;
  text: string;
}

export interface NotificationCreatedEvent {
  notification: Notification;
}

/** M9: a mission-level status transition, emitted by `orchestrator::scheduler::run_mission`. */
export interface MissionStatusChangedEvent {
  missionId: string;
  status: MissionStatus;
}

/**
 * M9: one task belonging to a running mission changed status. `reason` is
 * only set when the scheduler itself determined *why* (a dependency cycle,
 * or a failed upstream dependency) — it isn't persisted on the `tasks` row,
 * just surfaced live, so it should be treated as ephemeral (gone on reload).
 */
export interface MissionTaskUpdatedEvent {
  missionId: string;
  taskId: string;
  status: TaskStatus;
  reason: string | null;
}

export interface TerminalOutputEvent {
  terminalId: string;
  chunk: string;
}

export interface TerminalExitEvent {
  terminalId: string;
  exitCode: number | null;
}

/** Event-name -> payload-type map, used by `src/lib/events.ts`'s typed `listen()` wrapper. */
export interface ForgeEventMap {
  "agent:status-changed": AgentStatusChangedEvent;
  "agent-run:status-changed": AgentRunStatusChangedEvent;
  "agent-run:activity": AgentRunActivityEvent;
  "agent-run:tool-call-updated": ToolCallUpdatedEvent;
  "agent-run:message-delta": AgentRunMessageDeltaEvent;
  "notification:created": NotificationCreatedEvent;
  "mission:status-changed": MissionStatusChangedEvent;
  "mission:task-updated": MissionTaskUpdatedEvent;
  "terminal:output": TerminalOutputEvent;
  "terminal:exit": TerminalExitEvent;
}

export type ForgeEventName = keyof ForgeEventMap;

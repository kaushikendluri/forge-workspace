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
  Notification,
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

export interface NotificationCreatedEvent {
  notification: Notification;
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
  "notification:created": NotificationCreatedEvent;
  "terminal:output": TerminalOutputEvent;
  "terminal:exit": TerminalExitEvent;
}

export type ForgeEventName = keyof ForgeEventMap;

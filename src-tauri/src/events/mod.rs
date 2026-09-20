//! Typed wrappers around `tauri::Emitter::emit` for events the frontend
//! listens for via `src/lib/events.ts`'s `onForgeEvent`. Event names here
//! must stay in sync with `src/types/events.ts`'s `ForgeEventMap`.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::AppResult;

pub const AGENT_STATUS_CHANGED: &str = "agent:status-changed";
pub const AGENT_RUN_STATUS_CHANGED: &str = "agent-run:status-changed";
pub const AGENT_RUN_ACTIVITY: &str = "agent-run:activity";
pub const AGENT_RUN_TOOL_CALL_UPDATED: &str = "agent-run:tool-call-updated";
/// Ephemeral streamed assistant text (`content_block_delta` `text_delta`
/// chunks) — never persisted, purely a live "typing" feed for the UI. A
/// `model_message` `AGENT_RUN_ACTIVITY` event carries the final, persisted
/// text once the turn completes.
pub const AGENT_RUN_MESSAGE_DELTA: &str = "agent-run:message-delta";
pub const NOTIFICATION_CREATED: &str = "notification:created";
/// M9: a mission-level status transition (`planning` -> ... ->
/// `running` -> `completed`/`failed`/`stopped`), emitted by
/// `orchestrator::scheduler::run_mission`.
pub const MISSION_STATUS_CHANGED: &str = "mission:status-changed";
/// M9: one task belonging to a running mission changed status (picked up,
/// finished, blocked, cancelled), emitted alongside the real `tasks` row
/// write so `Tasks.tsx` can reflect it live without polling.
pub const MISSION_TASK_UPDATED: &str = "mission:task-updated";
pub const TERMINAL_OUTPUT: &str = "terminal:output";
pub const TERMINAL_EXIT: &str = "terminal:exit";

/// Emits `event` with `payload` to every window. Thin wrapper so call sites
/// in `commands/*.rs` don't each need to know about `tauri::Emitter`.
pub fn emit<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) -> AppResult<()> {
    app.emit(event, payload).map_err(|e| crate::error::AppError::Other(e.to_string()))
}

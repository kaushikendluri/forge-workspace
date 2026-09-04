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
pub const NOTIFICATION_CREATED: &str = "notification:created";
pub const TERMINAL_OUTPUT: &str = "terminal:output";
pub const TERMINAL_EXIT: &str = "terminal:exit";

/// Emits `event` with `payload` to every window. Thin wrapper so call sites
/// in `commands/*.rs` don't each need to know about `tauri::Emitter`.
pub fn emit<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) -> AppResult<()> {
    app.emit(event, payload).map_err(|e| crate::error::AppError::Other(e.to_string()))
}

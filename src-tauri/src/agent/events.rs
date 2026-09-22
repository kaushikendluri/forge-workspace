//! Typed event-emission helpers for the agent run loop, on top of
//! `crate::events`'s string constants. Kept in one place so `tool_loop.rs`
//! and `executor.rs` agree on exactly one payload shape per event — each
//! mirrors the corresponding type in `src/types/events.ts`.

use serde::Serialize;
use tauri::AppHandle;

use crate::db::models::{ActivityEvent, AgentRunStatus, AgentStatus, Notification, Review, ToolCall};
use crate::error::AppResult;
use crate::events;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AgentRunStatusChangedPayload<'a> {
    agent_run_id: &'a str,
    agent_id: &'a str,
    status: AgentRunStatus,
}

/// Mirrors `AgentRunStatusChangedEvent` in `src/types/events.ts`.
pub fn run_status_changed(app: &AppHandle, agent_run_id: &str, agent_id: &str, status: AgentRunStatus) -> AppResult<()> {
    events::emit(
        app,
        events::AGENT_RUN_STATUS_CHANGED,
        AgentRunStatusChangedPayload { agent_run_id, agent_id, status },
    )
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AgentStatusChangedPayload<'a> {
    agent_id: &'a str,
    status: AgentStatus,
}

/// Mirrors `AgentStatusChangedEvent` in `src/types/events.ts` — kept in sync
/// with every `agent_runs` status transition so `Agents.tsx`'s list reflects
/// a live run without a separate poll.
pub fn agent_status_changed(app: &AppHandle, agent_id: &str, status: AgentStatus) -> AppResult<()> {
    events::emit(app, events::AGENT_STATUS_CHANGED, AgentStatusChangedPayload { agent_id, status })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ActivityPayload<'a> {
    agent_run_id: &'a str,
    event: ActivityEvent,
}

/// Mirrors `AgentRunActivityEvent` in `src/types/events.ts`.
pub fn activity(app: &AppHandle, agent_run_id: &str, event: ActivityEvent) -> AppResult<()> {
    events::emit(app, events::AGENT_RUN_ACTIVITY, ActivityPayload { agent_run_id, event })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ToolCallUpdatedPayload<'a> {
    agent_run_id: &'a str,
    tool_call: ToolCall,
}

/// Mirrors `ToolCallUpdatedEvent` in `src/types/events.ts` — emitted once for
/// the `running` insert and again for the `success`/`error` completion of
/// every tool call.
pub fn tool_call_updated(app: &AppHandle, agent_run_id: &str, tool_call: ToolCall) -> AppResult<()> {
    events::emit(app, events::AGENT_RUN_TOOL_CALL_UPDATED, ToolCallUpdatedPayload { agent_run_id, tool_call })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MessageDeltaPayload<'a> {
    agent_run_id: &'a str,
    text: &'a str,
}

/// Mirrors `AgentRunMessageDeltaEvent` in `src/types/events.ts` — ephemeral,
/// never persisted (see `crate::events::AGENT_RUN_MESSAGE_DELTA`'s docs).
pub fn message_delta(app: &AppHandle, agent_run_id: &str, text: &str) -> AppResult<()> {
    events::emit(app, events::AGENT_RUN_MESSAGE_DELTA, MessageDeltaPayload { agent_run_id, text })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NotificationCreatedPayload {
    notification: Notification,
}

/// Mirrors `NotificationCreatedEvent` in `src/types/events.ts` — emitted
/// once, right after `db::repository::notifications::insert`, so
/// `TopBar.tsx`'s bell badge updates live without a navigate-away-and-back.
pub fn notification_created(app: &AppHandle, notification: Notification) -> AppResult<()> {
    events::emit(app, events::NOTIFICATION_CREATED, NotificationCreatedPayload { notification })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ReviewUpdatedPayload<'a> {
    agent_run_id: &'a str,
    review: Review,
}

/// M14: mirrors `ReviewUpdatedEvent` in `src/types/events.ts` — emitted once
/// by `agent::reviewer::run_review` right after its `reviews` row reaches a
/// terminal (`passed`/`failed`) status, so `AgentDetail.tsx`'s review panel
/// and `Tasks.tsx`'s board update live.
pub fn review_updated(app: &AppHandle, agent_run_id: &str, review: Review) -> AppResult<()> {
    events::emit(app, events::REVIEW_UPDATED, ReviewUpdatedPayload { agent_run_id, review })
}

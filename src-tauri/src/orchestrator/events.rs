//! Typed event-emission helpers for M9's scheduler, on the same pattern as
//! `agent::events` — kept in one place so `scheduler.rs` and
//! `commands::mission_commands` agree on exactly one payload shape per
//! event, each mirroring the corresponding type in `src/types/events.ts`.

use serde::Serialize;
use tauri::AppHandle;

use crate::db::models::{MissionStatus, TaskStatus};
use crate::error::AppResult;
use crate::events;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MissionStatusChangedPayload<'a> {
    mission_id: &'a str,
    status: MissionStatus,
}

/// Mirrors `MissionStatusChangedEvent` in `src/types/events.ts`.
pub fn mission_status_changed(app: &AppHandle, mission_id: &str, status: MissionStatus) -> AppResult<()> {
    events::emit(app, events::MISSION_STATUS_CHANGED, MissionStatusChangedPayload { mission_id, status })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MissionTaskUpdatedPayload<'a> {
    mission_id: &'a str,
    task_id: &'a str,
    status: TaskStatus,
    /// Set only when the scheduler itself determined *why* (e.g. a
    /// dependency cycle, or a failed upstream dependency) — not persisted on
    /// the `tasks` row itself (no schema column for it), just surfaced live
    /// so the UI can show an honest reason without inventing one.
    reason: Option<&'a str>,
}

/// Mirrors `MissionTaskUpdatedEvent` in `src/types/events.ts`.
pub fn mission_task_updated(
    app: &AppHandle,
    mission_id: &str,
    task_id: &str,
    status: TaskStatus,
    reason: Option<&str>,
) -> AppResult<()> {
    events::emit(app, events::MISSION_TASK_UPDATED, MissionTaskUpdatedPayload { mission_id, task_id, status, reason })
}

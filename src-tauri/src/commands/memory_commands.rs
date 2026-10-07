//! Phase 5 M18 commands: reading an agent's accumulated memory. `AgentMemory`
//! (from `db::models`) is returned directly as the frontend-facing DTO —
//! it already mirrors the `agent_memory` row 1:1 (see
//! `commands::agent_commands`'s own docs on when a separate `*Dto` wrapper
//! is/isn't needed), unlike `ProjectBrainDto`, which has to parse several
//! `*_json` columns into real typed values first.

use tauri::{AppHandle, Manager};

use crate::commands::run_blocking;
use crate::db::models::AgentMemory;
use crate::db::repository::agent_memory as agent_memory_repo;
use crate::error::AppResult;
use crate::state::AppState;

/// All of `agent_id`'s accumulated memory, most recently created first —
/// the full history, not the small retrieval-ranked subset a run's own
/// prompt gets (`agent::memory::rank_relevant_memories`); this is the
/// honest "everything this agent has learned so far" view for a UI panel,
/// an empty `Vec` if it has never completed a run yet.
#[tauri::command]
pub async fn list_agent_memory(app: AppHandle, agent_id: String) -> Result<Vec<AgentMemory>, String> {
    run_blocking(move || -> AppResult<Vec<AgentMemory>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        agent_memory_repo::list_for_agent(&conn, &agent_id)
    })
    .await
}

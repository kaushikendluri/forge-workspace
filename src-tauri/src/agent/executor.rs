//! Executes one `tool_use` block end-to-end: inserts/finalizes its
//! `tool_calls` row, mirrors that lifecycle into `activity_events`, emits
//! both live (`agent::events::tool_call_updated` /
//! `agent::events::activity`), and dispatches the actual tool logic via
//! `agent::tools::dispatch_tool`. `agent::tool_loop` calls this once per
//! `tool_use` block in a turn, sequentially.

use std::time::Instant;

use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::db::models::{ActivityEventType, ToolCallStatus};
use crate::db::repository::{activity_events as activity_events_repo, tool_calls as tool_calls_repo};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

use super::anthropic_client::ContentBlockParam;
use super::events as agent_events;
use super::tools::{dispatch_tool, elapsed_ms, ToolContext, ToolRunOutcome};

/// What executing one `tool_use` block produced, for `tool_loop` to fold
/// back into the next request's message history — or, for
/// `report_completion`, to end the run on.
pub enum ExecutedTool {
    ToolResult {
        block: ContentBlockParam,
        is_error: bool,
        /// M13: the `test_runs` row id this call persisted, if it was a
        /// `run_tests` call — see `tools::ToolRunOutcome::Result`.
        test_run_id: Option<String>,
    },
    Completion { summary: String, success: bool },
}

/// Runs one `tool_use` block. `sequence_number` is the run-wide, strictly
/// increasing call index (starting at 1) that orders `tool_calls` rows and
/// the activity feed.
pub async fn run_one_tool_call(
    app: &AppHandle,
    ctx: &ToolContext<'_>,
    agent_run_id: &str,
    sequence_number: i64,
    tool_use_id: &str,
    tool_name: &str,
    input: &Value,
) -> AppResult<ExecutedTool> {
    let input_json = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());

    let running_call = {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let call =
            tool_calls_repo::insert_running(&conn, agent_run_id, sequence_number, tool_use_id, tool_name, &input_json)?;
        let payload = serde_json::json!({ "toolName": tool_name, "input": input }).to_string();
        let event = activity_events_repo::insert(&conn, agent_run_id, Some(&call.id), ActivityEventType::ToolCallStarted, &payload)?;
        agent_events::activity(app, agent_run_id, event)?;
        call
    };
    agent_events::tool_call_updated(app, agent_run_id, running_call.clone())?;

    let start = Instant::now();
    let outcome = dispatch_tool(ctx, tool_name, input).await;
    let duration_ms = elapsed_ms(start);

    match outcome {
        ToolRunOutcome::Result { output, is_error, test_run_id } => {
            let status = if is_error { ToolCallStatus::Error } else { ToolCallStatus::Success };
            let error_message = if is_error { Some(output.as_str()) } else { None };
            let finished_call = {
                let state = app.state::<AppState>();
                let conn = state.db.get()?;
                tool_calls_repo::complete(&conn, &running_call.id, status, &output, error_message, duration_ms)?;
                let call = tool_calls_repo::get_by_id(&conn, &running_call.id)?
                    .ok_or_else(|| AppError::NotFound(format!("tool call {} vanished after completion", running_call.id)))?;
                let payload = serde_json::json!({ "toolName": tool_name, "isError": is_error, "output": output }).to_string();
                let event =
                    activity_events_repo::insert(&conn, agent_run_id, Some(&call.id), ActivityEventType::ToolCallCompleted, &payload)?;
                agent_events::activity(app, agent_run_id, event)?;
                call
            };
            agent_events::tool_call_updated(app, agent_run_id, finished_call)?;
            Ok(ExecutedTool::ToolResult {
                block: ContentBlockParam::ToolResult { tool_use_id: tool_use_id.to_string(), content: output, is_error },
                is_error,
                test_run_id,
            })
        }
        ToolRunOutcome::Completion { summary, success } => {
            let finished_call = {
                let state = app.state::<AppState>();
                let conn = state.db.get()?;
                tool_calls_repo::complete(&conn, &running_call.id, ToolCallStatus::Success, &summary, None, duration_ms)?;
                let call = tool_calls_repo::get_by_id(&conn, &running_call.id)?
                    .ok_or_else(|| AppError::NotFound(format!("tool call {} vanished after completion", running_call.id)))?;
                let payload = serde_json::json!({ "toolName": tool_name, "summary": summary, "success": success }).to_string();
                let event =
                    activity_events_repo::insert(&conn, agent_run_id, Some(&call.id), ActivityEventType::ToolCallCompleted, &payload)?;
                agent_events::activity(app, agent_run_id, event)?;
                call
            };
            agent_events::tool_call_updated(app, agent_run_id, finished_call)?;
            Ok(ExecutedTool::Completion { summary, success })
        }
    }
}

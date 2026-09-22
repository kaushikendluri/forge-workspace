//! M14 commands: triggering and reading back a reviewer run
//! (`agent::reviewer::run_review`) for an already-`completed` agent run. See
//! `agent::reviewer`'s own docs for what a "reviewer run" actually is (a
//! bounded, read-only context-gathering loop plus one forced structured
//! `submit_review` call) and why it reuses M6's tool machinery restricted to
//! read-only tools rather than a new engine.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::agent::reviewer;
use crate::commands::run_blocking;
use crate::db::models::{Review, ReviewFinding, ReviewStatus};
use crate::db::repository::reviews as reviews_repo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// A `Review` row with `findingsJson` parsed back into a real, structured
/// `findings` array — the shape `AgentDetail.tsx`'s review panel and
/// `Tasks.tsx`'s board actually consume. Mirrors `src/types/db.ts`'s
/// `ReviewDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDto {
    pub id: String,
    pub agent_run_id: String,
    pub score: i64,
    pub findings: Vec<ReviewFinding>,
    pub status: ReviewStatus,
    pub created_at: String,
}

fn to_dto(review: Review) -> AppResult<ReviewDto> {
    let findings: Vec<ReviewFinding> = serde_json::from_str(&review.findings_json)
        .map_err(|e| AppError::Other(format!("failed to parse stored review findings: {e}")))?;
    Ok(ReviewDto {
        id: review.id,
        agent_run_id: review.agent_run_id,
        score: review.score,
        findings,
        status: review.status,
        created_at: review.created_at,
    })
}

/// Runs a real reviewer pass for `agent_run_id` (which must already be
/// `completed`) end to end and returns the persisted review — score,
/// findings, and pass/fail status, all from a real structured API call.
/// Fails the same honest way `start_agent_run` does when no Anthropic API
/// key is configured; never fabricates a score.
#[tauri::command]
pub async fn request_review(app: AppHandle, agent_run_id: String) -> Result<ReviewDto, String> {
    let review = reviewer::run_review(&app, &agent_run_id).await.map_err(|e| e.to_string())?;
    to_dto(review).map_err(|e| e.to_string())
}

/// The latest review for `agent_run_id`, if any has ever completed — `None`
/// both when a review was never requested and when the most recent attempt
/// errored out before completing (see `agent::reviewer::run_review`'s docs
/// on why a failed attempt never leaves a stuck row behind). A review
/// currently in flight (auto-triggered by the scheduler, or a manual
/// `request_review` still running) shows up here with `status: "pending"`.
#[tauri::command]
pub async fn get_review(app: AppHandle, agent_run_id: String) -> Result<Option<ReviewDto>, String> {
    run_blocking(move || -> AppResult<Option<ReviewDto>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        match reviews_repo::get_latest_for_run(&conn, &agent_run_id)? {
            Some(review) => Ok(Some(to_dto(review)?)),
            None => Ok(None),
        }
    })
    .await
}

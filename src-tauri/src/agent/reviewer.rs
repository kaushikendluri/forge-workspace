//! M14: the reviewer agent. A reviewer run is a NEW *kind* of run — not a
//! new engine — built out of mostly the same machinery `agent::tool_loop`
//! already drives a normal M6 run with: the same `AnthropicClient`, the same
//! `agent::tools::dispatch_tool`, the same `ToolContext`. What's different is
//! the tool list (`agent::schema::reviewer_tool_definitions` — read-only, see
//! its own docs and the `reviewer_tool_list_excludes_every_mutating_tool`
//! test) and what the run ends on: not a `report_completion` call the model
//! chooses to make, but one forced `submit_review` structured-output call.
//!
//! ## Design choice: two phases, not a forced-tool-ending loop
//!
//! A reviewer needs to (1) look at a little real context — the diff, maybe a
//! file or two — and then (2) produce one structured verdict. Rather than
//! folding a `submit_review`-forces-the-end special case into
//! `agent::tool_loop`'s own loop (M6's loop ends on the model's own choice
//! to call `report_completion`; a reviewer has no equivalent "I'm done"
//! tool), this runs as two straightforward phases:
//!
//!   1. [`gather_context`]: an ordinary, bounded ([`MAX_CONTEXT_ITERATIONS`])
//!      multi-turn loop offering only the read-only tools, with **no**
//!      forced `tool_choice` — the model calls as many (or as few) of
//!      `git_status`/`git_diff`/`read_file`/`search_code`/... as it wants,
//!      stopping naturally once it stops calling tools (or the iteration cap
//!      is hit).
//!   2. [`request_review_submission`]: one final
//!      `AnthropicClient::request_structured_tool_call` (the same M8 planner
//!      pattern — see `orchestrator::planner`) with `submit_review` forced,
//!      handing back the accumulated conversation (every tool result from
//!      phase 1 included) so the verdict is grounded in what was actually
//!      read.
//!
//! This is simpler than a unified loop (no special-casing needed inside
//! `tool_loop.rs` itself, which stays untouched) while still guaranteeing the
//! model saw real repository content before scoring — never a fabricated
//! score.
//!
//! ## Score, pass threshold, and follow-up tasks
//!
//! `score` is 0-100 (100 = flawless). [`PASS_THRESHOLD`] (70) is the
//! pass/fail cutoff persisted as the review's `status`. Any finding with
//! [`ReviewSeverity::High`] or [`ReviewSeverity::Critical`]
//! ([`should_create_follow_up`]) gets a real follow-up `tasks` row — but only
//! when the reviewed run belongs to a mission task (`create_follow_up_tasks`)
//! since a solo run (started directly from the Agents page) has no mission
//! to file a follow-up into.
//!
//! ## What happens to a task whose review fails
//!
//! Nothing beyond the follow-up tasks above — a `failed` review does **not**
//! re-block or un-complete the task. The task already reached `done`, real
//! work already happened, and Phase 4's actual merge/self-healing gating on
//! review outcomes is M15's job, not this milestone's. A `failed` review is
//! conservative but real feedback: it shows up on `AgentDetail.tsx`, surfaces
//! real follow-up tasks in the mission's backlog, and that's it.

use std::path::PathBuf;
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::db::models::{AgentRunStatus, Review, ReviewFinding, ReviewSeverity, ReviewStatus, Task, TaskPriority};
use crate::db::repository::{
    agent_runs as agent_runs_repo, agents as agents_repo, model_configs as model_configs_repo, reviews as reviews_repo,
    tasks as tasks_repo, workspaces as workspaces_repo,
};
use crate::db::DbConnection;
use crate::error::{AppError, AppResult};
use crate::git::{GitCliService, GitService};
use crate::os_adapter;
use crate::secrets;
use crate::state::AppState;

use super::anthropic_client::{AnthropicClient, ContentBlockParam, MessageParam, StreamOutcome, ToolDefinition};
use super::schema::reviewer_tool_definitions;
use super::tool_loop::to_content_block_param;
use super::tools::{dispatch_tool, MissionContext as ToolMissionContext, ToolContext, ToolRunOutcome};

/// Bounded the same way M13's test-fix cycle is bounded — a handful of real
/// tool calls is enough for a reviewer to ground itself (see the diff, check
/// a file or two); this is deliberately much smaller than
/// `agent::tool_loop`'s `DEFAULT_MAX_ITERATIONS` since a reviewer is not
/// doing open-ended work.
const MAX_CONTEXT_ITERATIONS: i64 = 6;
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_TOKENS: u32 = 8192;
const SUBMIT_REVIEW_TOOL: &str = "submit_review";

/// Out of 100. A review scoring at or above this is `passed`; below it is
/// `failed` — see [`review_status_for_score`]. Chosen as a reasonably strict
/// but not punitive bar: a change with only minor/style-level findings should
/// still clear it, one with real correctness/security problems should not.
pub const PASS_THRESHOLD: i64 = 70;

fn get_conn(app: &AppHandle) -> AppResult<DbConnection> {
    app.state::<AppState>().db.get().map_err(Into::into)
}

// ---------------------------------------------------------------------
// submit_review tool + structured parsing (pure — unit-tested without any
// API call, per this milestone's constraints).
// ---------------------------------------------------------------------

fn submit_review_tool() -> ToolDefinition {
    ToolDefinition {
        name: SUBMIT_REVIEW_TOOL.to_string(),
        description: "Submit your finished structured code review for the change you just investigated: an \
                       overall score out of 100 and a list of findings across the review categories. Call this \
                       exactly once, after you've actually looked at the real diff/files via the tools available to \
                       you — never before, and never guess."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "score": {
                    "type": "integer",
                    "description": "Overall quality score for this change, 0-100 (100 = flawless, no issues found).",
                    "minimum": 0,
                    "maximum": 100
                },
                "findings": {
                    "type": "array",
                    "description": "Every issue found, if any — an empty array is a valid (clean) review.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "category": {
                                "type": "string",
                                "enum": ["correctness", "security", "performance", "maintainability", "tests", "architecture", "style"]
                            },
                            "severity": {
                                "type": "string",
                                "enum": ["low", "medium", "high", "critical"]
                            },
                            "summary": { "type": "string", "description": "What's wrong, where, and why it matters." },
                            "file": { "type": "string", "description": "The file this finding is about, if any." },
                            "line": { "type": "integer", "description": "The line this finding is about, if any." }
                        },
                        "required": ["category", "severity", "summary"]
                    }
                }
            },
            "required": ["score", "findings"]
        }),
    }
}

/// The `submit_review` tool call's parsed input — the reviewer's full
/// verdict before it's persisted as a `Review` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewSubmission {
    pub score: i64,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
}

/// Parses a `submit_review` tool call's `input` JSON into a
/// [`ReviewSubmission`], rejecting an out-of-range score explicitly (rather
/// than silently clamping it, which would quietly misrepresent what the
/// model actually said). Split out from the API-calling code so it's fully
/// unit-testable against hand-built fixtures, per this milestone's
/// constraints (no live API key in CI).
pub fn parse_review_submission(input: &serde_json::Value) -> AppResult<ReviewSubmission> {
    let submission: ReviewSubmission =
        serde_json::from_value(input.clone()).map_err(|e| AppError::Other(format!("model returned a malformed review: {e}")))?;
    if !(0..=100).contains(&submission.score) {
        return Err(AppError::Other(format!(
            "model returned an out-of-range review score: {} (must be 0-100)",
            submission.score
        )));
    }
    Ok(submission)
}

/// Maps a review's score to its persisted `status` — see [`PASS_THRESHOLD`].
pub fn review_status_for_score(score: i64) -> ReviewStatus {
    if score >= PASS_THRESHOLD {
        ReviewStatus::Passed
    } else {
        ReviewStatus::Failed
    }
}

/// Which findings are worth filing a real follow-up task over: `high` or
/// `critical` severity. `low`/`medium` findings still show up in the review
/// itself (for a human to read on `AgentDetail.tsx`) but don't spawn
/// automatic work — the plan doesn't call for an aggressive gate here, just
/// visibility for the findings that matter most.
pub fn should_create_follow_up(severity: ReviewSeverity) -> bool {
    matches!(severity, ReviewSeverity::High | ReviewSeverity::Critical)
}

fn category_label(category: crate::db::models::ReviewCategory) -> &'static str {
    use crate::db::models::ReviewCategory::*;
    match category {
        Correctness => "correctness",
        Security => "security",
        Performance => "performance",
        Maintainability => "maintainability",
        Tests => "tests",
        Architecture => "architecture",
        Style => "style",
    }
}

/// A short, specific title for a finding's follow-up task — the finding's
/// full `summary` is kept verbatim as the task's description (not
/// duplicated/truncated here), this is just what a task list shows at a
/// glance. Capped so an unusually long model-provided summary can't produce
/// an unreadable title.
pub fn follow_up_task_title(finding: &ReviewFinding) -> String {
    let prefix = format!("Review finding ({}): ", category_label(finding.category));
    let max_summary_chars = 120usize.saturating_sub(prefix.chars().count());
    let mut summary: String = finding.summary.chars().take(max_summary_chars).collect();
    if summary.chars().count() < finding.summary.chars().count() {
        summary.push_str("…");
    }
    format!("{prefix}{summary}")
}

/// Creates one real follow-up `tasks` row (`tasks_repo::insert_for_mission`
/// — the same function M8/M9 already use, not a duplicate) per
/// [`should_create_follow_up`] finding in `submission`, when the reviewed
/// run belongs to a mission task. A solo run (`tasks_repo::get_by_agent_run_id`
/// returns `None`, or returns a task with no `mission_id`) has nowhere to
/// file a follow-up into, so this is a no-op for it — not an error. Pure DB
/// logic, split out from [`run_review`] specifically so it's unit-testable
/// against a seeded in-memory DB without any Anthropic API call.
fn create_follow_up_tasks(conn: &Connection, agent_run_id: &str, submission: &ReviewSubmission) -> AppResult<Vec<Task>> {
    let Some(task) = tasks_repo::get_by_agent_run_id(conn, agent_run_id)? else { return Ok(vec![]) };
    let Some(mission_id) = task.mission_id.clone() else { return Ok(vec![]) };

    let next_position = tasks_repo::list_for_mission(conn, &mission_id)?.len() as i64;
    let mut created = Vec::new();
    for (offset, finding) in submission.findings.iter().filter(|f| should_create_follow_up(f.severity)).enumerate() {
        let follow_up = tasks_repo::insert_for_mission(
            conn,
            &task.project_id,
            &mission_id,
            &follow_up_task_title(finding),
            Some(finding.summary.as_str()),
            "fix",
            TaskPriority::High,
            next_position + offset as i64,
        )?;
        created.push(follow_up);
    }
    Ok(created)
}

// ---------------------------------------------------------------------
// Phase 1: bounded, read-only context gathering.
// ---------------------------------------------------------------------

fn context_system_prompt(task_prompt: &str, workspace_root: &str) -> String {
    format!(
        "You are a meticulous, read-only code reviewer. You are reviewing the result of a coding agent's \
         completed task, inside a git worktree at `{workspace_root}`. Nothing outside this worktree is reachable \
         through your tools, and none of your tools can modify anything — you may only read files, list \
         directories, search code, and inspect git status/diff/log.\n\n\
         The task that was completed:\n{task_prompt}\n\n\
         Investigate the real change before forming an opinion: start with `git_status` to see what changed, then \
         `git_diff` on the changed files, and `read_file`/`search_code` for any surrounding context you need to \
         judge correctness, security, performance, maintainability, test coverage, architecture, and style. When \
         you've seen enough to score the change confidently, simply stop calling tools — you will then be asked to \
         submit a structured review.",
    )
}

/// Runs phase 1: a bounded, read-only multi-turn loop (no forced
/// `tool_choice`), returning the accumulated message history (including
/// every real tool result) for phase 2 to hand back to the model alongside
/// the forced `submit_review` call.
async fn gather_context(
    client: &AnthropicClient,
    model: &str,
    max_tokens: u32,
    system: &str,
    ctx: &ToolContext<'_>,
    cancel: &CancellationToken,
) -> AppResult<Vec<MessageParam>> {
    let tools = reviewer_tool_definitions();
    let mut messages = vec![MessageParam::user_text(
        "Investigate the change now using the read-only tools available to you, then stop calling tools once you've \
         seen enough to review it confidently.",
    )];

    for _ in 0..MAX_CONTEXT_ITERATIONS {
        if cancel.is_cancelled() {
            return Err(AppError::Other("review was cancelled".to_string()));
        }

        let outcome = client.stream_turn(model, max_tokens, system, &messages, &tools, cancel, |_text: &str| {}).await?;
        let turn = match outcome {
            StreamOutcome::Turn(turn) => turn,
            StreamOutcome::Cancelled => return Err(AppError::Other("review was cancelled".to_string())),
        };

        let tool_uses: Vec<(String, String, serde_json::Value)> =
            turn.tool_uses().map(|(id, name, input)| (id.to_string(), name.to_string(), input.clone())).collect();

        let assistant_blocks: Vec<ContentBlockParam> = turn.content.iter().map(to_content_block_param).collect();
        messages.push(MessageParam::assistant(assistant_blocks));

        if tool_uses.is_empty() {
            // The model stopped calling tools on its own — it has decided
            // it's seen enough context. Phase 2 asks it to submit the
            // review next.
            break;
        }

        let mut tool_results: Vec<ContentBlockParam> = Vec::with_capacity(tool_uses.len());
        for (tool_use_id, tool_name, input) in tool_uses {
            if cancel.is_cancelled() {
                return Err(AppError::Other("review was cancelled".to_string()));
            }
            let outcome = dispatch_tool(ctx, &tool_name, &input).await;
            let (output, is_error) = match outcome {
                ToolRunOutcome::Result { output, is_error, .. } => (output, is_error),
                // A reviewer is never offered `report_completion`/
                // `send_message` (see `reviewer_tool_definitions`), so this
                // arm is unreachable in practice — handled defensively
                // rather than panicking if that ever changes.
                ToolRunOutcome::Completion { summary, .. } => (summary, false),
            };
            tool_results.push(ContentBlockParam::ToolResult { tool_use_id, content: output, is_error });
        }
        messages.push(MessageParam::user_tool_results(tool_results));
    }

    Ok(messages)
}

// ---------------------------------------------------------------------
// Phase 2: one forced structured `submit_review` call.
// ---------------------------------------------------------------------

async fn request_review_submission(
    client: &AnthropicClient,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[MessageParam],
    cancel: &CancellationToken,
) -> AppResult<ReviewSubmission> {
    let tool = submit_review_tool();
    let outcome = client.request_structured_tool_call(model, max_tokens, system, messages, &tool, cancel).await?;

    let turn = match outcome {
        StreamOutcome::Turn(turn) => turn,
        StreamOutcome::Cancelled => return Err(AppError::Other("review was cancelled".to_string())),
    };

    let submission_input = turn.tool_uses().find(|(_, name, _)| *name == SUBMIT_REVIEW_TOOL).map(|(_, _, input)| input).ok_or_else(|| {
        let text = turn.text();
        if text.trim().is_empty() {
            AppError::Other(format!("the model did not call `{SUBMIT_REVIEW_TOOL}`"))
        } else {
            AppError::Other(format!("the model did not call `{SUBMIT_REVIEW_TOOL}` — it said: {text}"))
        }
    })?;

    parse_review_submission(submission_input)
}

// ---------------------------------------------------------------------
// The real entry point.
// ---------------------------------------------------------------------

/// Runs a real reviewer pass for `agent_run_id` end to end: loads the
/// (already `completed`) run and its workspace, checks for a configured API
/// key (failing the same honest way `agent::tool_loop::run_agent_loop`
/// already does for a normal run if none is configured — never a fake
/// score), runs phase 1 + phase 2 above, persists the result as a real
/// `reviews` row, files real follow-up tasks for high/critical findings
/// ([`create_follow_up_tasks`]), and emits `review:updated`.
///
/// A `reviews` row is inserted (`pending`) right before phase 1 starts, so a
/// task genuinely sits in the Kanban board's Review column while this is in
/// flight; if anything fails before a real verdict comes back, that row is
/// deleted rather than left stuck `pending` or fabricated into a fake
/// `failed` result — `get_review` then honestly reports "no review" again.
pub async fn run_review(app: &AppHandle, agent_run_id: &str) -> AppResult<Review> {
    let (agent_run, agent, workspace) = {
        let conn = get_conn(app)?;
        let agent_run = agent_runs_repo::get_by_id(&conn, agent_run_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))?;
        if agent_run.status != AgentRunStatus::Completed {
            return Err(AppError::InvalidInput(format!(
                "agent run {agent_run_id} has not completed successfully (status: {:?}) — only a completed run can be reviewed",
                agent_run.status
            )));
        }
        let agent = agents_repo::get_by_id(&conn, &agent_run.agent_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent {} not found", agent_run.agent_id)))?;
        let workspace_id = agent_run
            .workspace_id
            .clone()
            .ok_or_else(|| AppError::InvalidInput(format!("agent run {agent_run_id} has no workspace to review")))?;
        let workspace = workspaces_repo::get_by_id(&conn, &workspace_id)?
            .ok_or_else(|| AppError::NotFound(format!("workspace {workspace_id} not found")))?;
        (agent_run, agent, workspace)
    };

    // Same "fail loudly, never silently produce a fake result" pattern
    // `agent::tool_loop::run_agent_loop_inner` uses before an M6 run starts.
    let api_key = tauri::async_runtime::spawn_blocking(|| secrets::get_secret(secrets::ANTHROPIC_API_KEY))
        .await
        .map_err(|e| AppError::Other(format!("API key lookup panicked: {e}")))??;
    let Some(api_key) = api_key else {
        return Err(AppError::InvalidInput(
            "No Anthropic API key is configured. Add one in Settings, then request a review again.".to_string(),
        ));
    };

    let client = AnthropicClient::new(api_key)?;
    let max_tokens = {
        let conn = get_conn(app)?;
        model_configs_repo::list(&conn)?
            .into_iter()
            .find(|m| m.model_id == agent_run.model_id)
            .map(|m| m.max_output_tokens as u32)
            .unwrap_or(DEFAULT_MAX_TOKENS)
    };

    let os_adapter = os_adapter::current();
    let git_service: Box<dyn GitService> = Box::new(GitCliService::new(os_adapter.as_ref()));
    let workspace_root = PathBuf::from(&workspace.path);
    let cancel = CancellationToken::new();
    let db_pool = app.state::<AppState>().db.clone();

    let ctx = ToolContext {
        workspace_root,
        git_service: git_service.as_ref(),
        os_adapter: os_adapter.as_ref(),
        test_command: None,
        lint_command: None,
        build_command: None,
        tool_timeout: Duration::from_millis(DEFAULT_TOOL_TIMEOUT_MS),
        cancel: cancel.clone(),
        agent_run_id: agent_run_id.to_string(),
        // The reviewer never offers `send_message` (it's not in
        // `reviewer_tool_definitions`), so this is never consulted — `None`
        // either way, matching a solo (non-mission) M6 run's own default.
        mission_context: None::<ToolMissionContext>,
        db_pool,
        project_id: agent.project_id.clone(),
    };

    let system = context_system_prompt(&agent_run.task_prompt, &workspace.path);

    let pending = {
        let conn = get_conn(app)?;
        reviews_repo::insert_pending(&conn, agent_run_id)?
    };

    let submission_result: AppResult<ReviewSubmission> = async {
        let messages = gather_context(&client, &agent_run.model_id, max_tokens, &system, &ctx, &cancel).await?;
        request_review_submission(&client, &agent_run.model_id, max_tokens, &system, &messages, &cancel).await
    }
    .await;

    let submission = match submission_result {
        Ok(submission) => submission,
        Err(e) => {
            let conn = get_conn(app)?;
            let _ = reviews_repo::delete(&conn, &pending.id);
            return Err(e);
        }
    };

    let status = review_status_for_score(submission.score);
    let findings_json =
        serde_json::to_string(&submission.findings).map_err(|e| AppError::Other(format!("failed to serialize review findings: {e}")))?;

    let review = {
        let conn = get_conn(app)?;
        reviews_repo::complete(&conn, &pending.id, submission.score, &findings_json, status)?;
        create_follow_up_tasks(&conn, agent_run_id, &submission)?;
        reviews_repo::get_latest_for_run(&conn, agent_run_id)?
            .ok_or_else(|| AppError::NotFound(format!("review {} vanished after completion", pending.id)))?
    };

    let _ = super::events::review_updated(app, agent_run_id, review.clone());

    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use crate::db::models::ReviewCategory;

    fn finding(category: ReviewCategory, severity: ReviewSeverity, summary: &str) -> ReviewFinding {
        ReviewFinding { category, severity, summary: summary.to_string(), file: None, line: None }
    }

    // -- parse_review_submission (pure) -------------------------------------

    #[test]
    fn parses_a_well_formed_submission_with_findings() {
        let input = json!({
            "score": 62,
            "findings": [
                { "category": "security", "severity": "high", "summary": "SQL built via string concat", "file": "db.rs", "line": 42 },
                { "category": "style", "severity": "low", "summary": "inconsistent naming" }
            ]
        });
        let submission = parse_review_submission(&input).expect("should parse");
        assert_eq!(submission.score, 62);
        assert_eq!(submission.findings.len(), 2);
        assert_eq!(submission.findings[0].category, ReviewCategory::Security);
        assert_eq!(submission.findings[0].severity, ReviewSeverity::High);
        assert_eq!(submission.findings[0].file.as_deref(), Some("db.rs"));
        assert_eq!(submission.findings[0].line, Some(42));
        assert_eq!(submission.findings[1].file, None);
    }

    #[test]
    fn parses_a_clean_review_with_no_findings() {
        let input = json!({ "score": 100, "findings": [] });
        let submission = parse_review_submission(&input).expect("should parse");
        assert_eq!(submission.score, 100);
        assert!(submission.findings.is_empty());
    }

    #[test]
    fn missing_findings_field_defaults_to_empty_rather_than_erroring() {
        let input = json!({ "score": 90 });
        let submission = parse_review_submission(&input).expect("should parse");
        assert!(submission.findings.is_empty());
    }

    #[test]
    fn rejects_an_out_of_range_score() {
        let too_high = json!({ "score": 150, "findings": [] });
        assert!(parse_review_submission(&too_high).is_err());

        let negative = json!({ "score": -5, "findings": [] });
        assert!(parse_review_submission(&negative).is_err());
    }

    #[test]
    fn rejects_an_unknown_category_or_severity() {
        let bad_category = json!({ "score": 50, "findings": [{ "category": "vibes", "severity": "low", "summary": "x" }] });
        assert!(parse_review_submission(&bad_category).is_err());

        let bad_severity = json!({ "score": 50, "findings": [{ "category": "style", "severity": "extreme", "summary": "x" }] });
        assert!(parse_review_submission(&bad_severity).is_err());
    }

    #[test]
    fn missing_required_fields_fails_clearly() {
        let input = json!({ "findings": [] });
        let err = parse_review_submission(&input).expect_err("score is required");
        assert!(err.to_string().contains("malformed review"));
    }

    // -- review_status_for_score (pure) --------------------------------------

    #[test]
    fn score_at_or_above_threshold_passes() {
        assert_eq!(review_status_for_score(PASS_THRESHOLD), ReviewStatus::Passed);
        assert_eq!(review_status_for_score(100), ReviewStatus::Passed);
    }

    #[test]
    fn score_below_threshold_fails() {
        assert_eq!(review_status_for_score(PASS_THRESHOLD - 1), ReviewStatus::Failed);
        assert_eq!(review_status_for_score(0), ReviewStatus::Failed);
    }

    // -- should_create_follow_up (pure) --------------------------------------

    #[test]
    fn only_high_and_critical_severity_create_follow_ups() {
        assert!(!should_create_follow_up(ReviewSeverity::Low));
        assert!(!should_create_follow_up(ReviewSeverity::Medium));
        assert!(should_create_follow_up(ReviewSeverity::High));
        assert!(should_create_follow_up(ReviewSeverity::Critical));
    }

    // -- follow_up_task_title (pure) -----------------------------------------

    #[test]
    fn follow_up_title_includes_category_and_summary() {
        let f = finding(ReviewCategory::Security, ReviewSeverity::Critical, "hardcoded API key in source");
        let title = follow_up_task_title(&f);
        assert!(title.contains("security"));
        assert!(title.contains("hardcoded API key in source"));
    }

    #[test]
    fn follow_up_title_truncates_an_unusually_long_summary() {
        let long_summary = "x".repeat(500);
        let f = finding(ReviewCategory::Correctness, ReviewSeverity::High, &long_summary);
        let title = follow_up_task_title(&f);
        assert!(title.chars().count() < 200, "title should stay well short of the full 500-char summary");
        assert!(title.contains("correctness"));
    }

    // -- create_follow_up_tasks (DB logic, no API call) ----------------------

    fn migrated_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn
    }

    fn seed_mission_task_linked_to_run(conn: &Connection, run_id: &str, task_id: &str) {
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
        conn.execute("INSERT INTO missions (id, project_id, objective) VALUES ('m1', 'p1', 'Ship it')", []).unwrap();
        conn.execute(
            &format!("INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('{run_id}', 'a1', 'do it', 'claude-sonnet-5')"),
            [],
        )
        .unwrap();
        conn.execute(
            &format!(
                "INSERT INTO tasks (id, project_id, mission_id, title, status, priority, position, agent_run_id) \
                 VALUES ('{task_id}', 'p1', 'm1', 'Do the thing', 'done', 'medium', 0, '{run_id}')"
            ),
            [],
        )
        .unwrap();
    }

    #[test]
    fn creates_a_follow_up_task_for_each_high_or_critical_finding_only() {
        let conn = migrated_conn();
        seed_mission_task_linked_to_run(&conn, "run1", "t1");

        let submission = ReviewSubmission {
            score: 40,
            findings: vec![
                finding(ReviewCategory::Security, ReviewSeverity::Critical, "auth bypass"),
                finding(ReviewCategory::Style, ReviewSeverity::Low, "minor formatting nit"),
                finding(ReviewCategory::Correctness, ReviewSeverity::High, "off-by-one in pagination"),
            ],
        };

        let created = create_follow_up_tasks(&conn, "run1", &submission).expect("create_follow_up_tasks");
        assert_eq!(created.len(), 2, "only the high/critical findings should create follow-up tasks");
        for task in &created {
            assert_eq!(task.mission_id.as_deref(), Some("m1"));
            assert_eq!(task.status, crate::db::models::TaskStatus::Backlog);
            assert_eq!(task.priority, TaskPriority::High);
        }

        let mission_tasks = tasks_repo::list_for_mission(&conn, "m1").expect("list_for_mission");
        // The original task plus the two new follow-ups.
        assert_eq!(mission_tasks.len(), 3);
    }

    #[test]
    fn creates_no_follow_up_tasks_when_every_finding_is_low_or_medium_severity() {
        let conn = migrated_conn();
        seed_mission_task_linked_to_run(&conn, "run1", "t1");

        let submission = ReviewSubmission {
            score: 85,
            findings: vec![
                finding(ReviewCategory::Style, ReviewSeverity::Low, "nit"),
                finding(ReviewCategory::Tests, ReviewSeverity::Medium, "could use one more test case"),
            ],
        };

        let created = create_follow_up_tasks(&conn, "run1", &submission).expect("create_follow_up_tasks");
        assert!(created.is_empty());
    }

    #[test]
    fn creates_no_follow_up_tasks_for_a_solo_non_mission_run() {
        let conn = migrated_conn();
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .unwrap();
        // A standalone task (no mission) linked to the run — mirrors a task
        // created directly, not via a mission plan.
        let task = tasks_repo::create(&conn, "p1", "Standalone task", None).expect("create");
        tasks_repo::set_agent_run_id(&conn, &task.id, "run1").expect("set_agent_run_id");

        let submission = ReviewSubmission {
            score: 10,
            findings: vec![finding(ReviewCategory::Security, ReviewSeverity::Critical, "very bad")],
        };
        let created = create_follow_up_tasks(&conn, "run1", &submission).expect("create_follow_up_tasks");
        assert!(created.is_empty(), "a solo run has no mission to file a follow-up task into");
    }

    #[test]
    fn creates_no_follow_up_tasks_when_the_run_has_no_linked_task_at_all() {
        let conn = migrated_conn();
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .unwrap();

        let submission = ReviewSubmission { score: 10, findings: vec![finding(ReviewCategory::Security, ReviewSeverity::Critical, "bad")] };
        let created = create_follow_up_tasks(&conn, "run1", &submission).expect("create_follow_up_tasks");
        assert!(created.is_empty());
    }
}

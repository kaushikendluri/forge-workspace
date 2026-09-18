//! Turns a plain-English objective into a structured [`MissionPlan`] via one
//! forced-tool-use Anthropic Messages API call — not the M6 multi-turn
//! tool-calling agent loop, a single structured-output request. This module
//! only *proposes* a plan; `commands::mission_commands::create_mission`
//! persists it (resolving each task's plan-local `depends_on` id into a real
//! `tasks.depends_on_task_id` foreign key) and the user approves it before
//! anything executes (a later milestone).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::agent::anthropic_client::{AnthropicClient, MessageParam, StreamOutcome, ToolDefinition};
use crate::error::{AppError, AppResult};
use crate::git::GitService;

const PROPOSE_PLAN_TOOL: &str = "propose_plan";
/// A shallow top-level directory listing is enough to tell the model roughly
/// what kind of project this is (a `package.json`, a `Cargo.toml`, a `src/`
/// layout, ...) without walking the whole tree — this is deliberately not
/// the Project Brain (Phase 5).
const MAX_CONTEXT_ENTRIES: usize = 60;

/// One task as the model proposed it, before its `depends_on` plan-local id
/// has been resolved to a real `tasks.id` — that resolution happens once
/// each task has actually been inserted and has a real row (see
/// `commands::mission_commands::persist_plan`), since a task can depend on
/// another task declared later in the same plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedTask {
    /// An id unique only within this one plan (e.g. `"1"`, `"t1"`,
    /// `"setup-db"`) — used solely to express `depends_on` edges between
    /// tasks in the same response. Never stored as-is; each task gets a real
    /// UUID when its `tasks` row is inserted.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub agent_type: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    /// The `id` of another task in this same plan that must finish first, if
    /// any.
    #[serde(default)]
    pub depends_on: Option<String>,
}

fn default_priority() -> String {
    "medium".to_string()
}

/// The full structured plan for one mission's objective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionPlan {
    pub tasks: Vec<PlannedTask>,
}

/// The `propose_plan` tool definition sent as the request's (only, forced)
/// tool — its `input_schema` is the actual contract for what a "structured
/// task plan" means to this planner.
fn propose_plan_tool() -> ToolDefinition {
    ToolDefinition {
        name: PROPOSE_PLAN_TOOL.to_string(),
        description: "Propose a structured task plan for the given objective: an ordered list of tasks, each \
                       assigned a suggested specialist agent type, with explicit dependencies between tasks in \
                       this same plan."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "The proposed tasks, in a sensible execution order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "A short id unique within this plan (e.g. '1', 't1'), used only to express `depends_on` below."
                            },
                            "title": { "type": "string", "description": "A short, specific task title." },
                            "description": {
                                "type": "string",
                                "description": "What this task involves and how to tell it's done."
                            },
                            "agent_type": {
                                "type": "string",
                                "description": "The suggested specialist role for this task, e.g. 'frontend', 'backend', 'database', 'qa', 'devops'."
                            },
                            "priority": { "type": "string", "enum": ["low", "medium", "high"] },
                            "depends_on": {
                                "type": "string",
                                "description": "The `id` of another task in this plan that must complete before this one can start. Omit if this task has no dependency."
                            }
                        },
                        "required": ["id", "title", "agent_type"]
                    }
                }
            },
            "required": ["tasks"]
        }),
    }
}

fn system_prompt() -> &'static str {
    "You are a technical project planner. Given a plain-English objective and a short repository summary, break \
     the objective down into a concrete, ordered list of tasks that a team of specialist coding agents could \
     execute. Each task should be scoped to one clear piece of work, assigned a suggested specialist agent type \
     (e.g. 'frontend', 'backend', 'database', 'qa', 'devops', or another short role name that fits) and a \
     priority. Express real dependencies between tasks explicitly via `depends_on` — most plans are either a \
     short dependency chain or a small number of independent, parallelizable tasks; do not invent a dependency \
     that isn't necessary. Call `propose_plan` exactly once with the full task list, even if that list has just \
     one task."
}

/// A minimal repo-context summary to ground the plan: the repository's
/// folder name, current branch, last 5 commits, and a shallow top-level
/// file listing. Deliberately shallow — this is not the full Project Brain
/// (Phase 5), just enough that the model isn't planning completely blind.
fn build_repo_context(repo_root: &Path, git: &dyn GitService) -> String {
    let folder_name =
        repo_root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| repo_root.to_string_lossy().into_owned());

    let branch = git.current_branch(repo_root).ok().flatten().unwrap_or_else(|| "(unknown)".to_string());

    let commits = git.log(repo_root, 5).unwrap_or_default();
    let commits_summary = if commits.is_empty() {
        "(no commits yet)".to_string()
    } else {
        commits.iter().map(|c| format!("- {} {}", c.short_sha, c.subject)).collect::<Vec<_>>().join("\n")
    };

    let listing = shallow_top_level_listing(repo_root);

    format!(
        "Repository folder: {folder_name}\nCurrent branch: {branch}\nRecent commits (most recent first):\n\
         {commits_summary}\n\nTop-level contents:\n{listing}"
    )
}

/// A non-recursive listing of `repo_root`'s immediate entries (directories
/// suffixed `/`) — the same shallow granularity as the Files tab's directory
/// listing (`commands::fs_commands::list_directory`), capped rather than a
/// full recursive repository walk.
fn shallow_top_level_listing(repo_root: &Path) -> String {
    let Ok(read_dir) = std::fs::read_dir(repo_root) else {
        return "(unable to list repository contents)".to_string();
    };
    let mut entries: Vec<String> = read_dir
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(format!("{name}{}", if is_dir { "/" } else { "" }))
        })
        .take(MAX_CONTEXT_ENTRIES)
        .collect();
    entries.sort();
    if entries.is_empty() {
        "(empty)".to_string()
    } else {
        entries.join("\n")
    }
}

/// Runs the one Anthropic call that turns `objective` into a [`MissionPlan`],
/// grounded by a shallow summary of the repository at `repo_root`. Returns
/// `Err` for any genuine failure (API error, no API key having already been
/// checked by the caller, the model not calling `propose_plan`, a malformed
/// plan) — the caller (`commands::mission_commands::create_mission`) is
/// responsible for turning that into a `missions.status = 'failed'` row
/// rather than silently producing an empty plan.
pub async fn propose_plan(
    client: &AnthropicClient,
    model: &str,
    max_tokens: u32,
    repo_root: &Path,
    git: &dyn GitService,
    objective: &str,
    cancel: &CancellationToken,
) -> AppResult<MissionPlan> {
    let repo_context = build_repo_context(repo_root, git);
    let user_message = format!("Objective:\n{objective}\n\nRepository context:\n{repo_context}");
    let messages = vec![MessageParam::user_text(user_message)];
    let tool = propose_plan_tool();

    let outcome =
        client.request_structured_tool_call(model, max_tokens, system_prompt(), &messages, &tool, cancel).await?;

    let turn = match outcome {
        StreamOutcome::Turn(turn) => turn,
        StreamOutcome::Cancelled => return Err(AppError::Other("mission planning was cancelled".to_string())),
    };

    let plan_input = turn
        .tool_uses()
        .find(|(_, name, _)| *name == PROPOSE_PLAN_TOOL)
        .map(|(_, _, input)| input)
        .ok_or_else(|| {
            let text = turn.text();
            if text.trim().is_empty() {
                AppError::Other(format!("the model did not call `{PROPOSE_PLAN_TOOL}`"))
            } else {
                AppError::Other(format!("the model did not call `{PROPOSE_PLAN_TOOL}` — it said: {text}"))
            }
        })?;

    parse_mission_plan(plan_input)
}

/// Parses a `propose_plan` tool call's `input` JSON into a [`MissionPlan`].
/// Split out from `propose_plan` so it can be unit-tested against
/// hand-built fixtures without a live API call — see the tests below.
pub fn parse_mission_plan(input: &serde_json::Value) -> AppResult<MissionPlan> {
    serde_json::from_value(input.clone()).map_err(|e| AppError::Other(format!("model returned a malformed plan: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple linear plan: three tasks, each depending on the previous
    /// one — the most common shape a real objective should produce.
    #[test]
    fn parses_a_simple_linear_dependency_chain() {
        let input = json!({
            "tasks": [
                { "id": "1", "title": "Design the schema", "agent_type": "database", "priority": "high" },
                { "id": "2", "title": "Build the API", "agent_type": "backend", "priority": "medium", "depends_on": "1" },
                { "id": "3", "title": "Wire up the UI", "agent_type": "frontend", "priority": "medium", "depends_on": "2" }
            ]
        });

        let plan = parse_mission_plan(&input).expect("should parse a well-formed plan");
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].depends_on, None);
        assert_eq!(plan.tasks[1].depends_on.as_deref(), Some("1"));
        assert_eq!(plan.tasks[2].depends_on.as_deref(), Some("2"));
        assert_eq!(plan.tasks[0].priority, "high");
        assert_eq!(plan.tasks[0].agent_type, "database");
    }

    /// A plan with a real dependency *graph* rather than a straight chain:
    /// two independent setup tasks that a later task depends on both of —
    /// this module's `depends_on` field only models a single predecessor
    /// per task (matching the `tasks.depends_on_task_id` single-FK column
    /// it's persisted into), so this fixture checks that a plan naming only
    /// one of the two predecessors still parses cleanly rather than
    /// erroring over the modeling limitation.
    #[test]
    fn parses_a_plan_with_multiple_predecessors_converging_on_one_task() {
        let input = json!({
            "tasks": [
                { "id": "backend-setup", "title": "Scaffold the backend service", "agent_type": "backend" },
                { "id": "db-setup", "title": "Provision the database", "agent_type": "database" },
                {
                    "id": "integration",
                    "title": "Wire the backend to the database",
                    "description": "Connect the scaffolded service to the provisioned database.",
                    "agent_type": "backend",
                    "priority": "high",
                    "depends_on": "db-setup"
                }
            ]
        });

        let plan = parse_mission_plan(&input).expect("should parse");
        assert_eq!(plan.tasks.len(), 3);
        let integration = plan.tasks.iter().find(|t| t.id == "integration").expect("integration task present");
        assert_eq!(integration.depends_on.as_deref(), Some("db-setup"));
        assert_eq!(integration.description.as_deref(), Some("Connect the scaffolded service to the provisioned database."));
    }

    /// A plan with one isolated/parallel task alongside a dependency chain
    /// — e.g. writing docs, which doesn't block or get blocked by anything.
    /// Also exercises the `priority`/`description` defaults: the model is
    /// allowed to omit both.
    #[test]
    fn parses_a_plan_with_an_isolated_parallel_task_and_missing_optional_fields() {
        let input = json!({
            "tasks": [
                { "id": "1", "title": "Implement the feature", "agent_type": "backend", "depends_on": null },
                { "id": "2", "title": "Write the changelog entry", "agent_type": "docs" }
            ]
        });

        let plan = parse_mission_plan(&input).expect("should parse a plan with omitted optional fields");
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].depends_on, None);
        assert_eq!(plan.tasks[1].depends_on, None, "a task with no depends_on field at all should default to None");
        assert_eq!(plan.tasks[1].priority, "medium", "priority should default when the model omits it");
        assert_eq!(plan.tasks[1].description, None);
    }

    #[test]
    fn empty_task_list_parses_rather_than_erroring() {
        // The model proposing zero tasks shouldn't normally happen, but the
        // parser (and everything downstream of it) must handle it honestly
        // rather than panicking — see `commands::mission_commands`'s
        // handling of a zero-task `MissionPlan`.
        let input = json!({ "tasks": [] });
        let plan = parse_mission_plan(&input).expect("an empty task list is a valid (if unusual) plan");
        assert!(plan.tasks.is_empty());
    }

    #[test]
    fn missing_required_field_fails_clearly_rather_than_silently_dropping_the_task() {
        // `agent_type` is required by the schema; a plan missing it for one
        // task should fail parsing with a real error rather than silently
        // producing a plan with fewer tasks than the model proposed.
        let input = json!({
            "tasks": [
                { "id": "1", "title": "Do something" }
            ]
        });
        let err = parse_mission_plan(&input).expect_err("missing agent_type should fail to parse");
        assert!(err.to_string().contains("malformed plan"));
    }

    #[test]
    fn missing_tasks_field_entirely_fails_clearly() {
        let input = json!({ "not_tasks": [] });
        let err = parse_mission_plan(&input).expect_err("a response with no `tasks` field should fail to parse");
        assert!(err.to_string().contains("malformed plan"));
    }
}

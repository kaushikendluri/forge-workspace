//! Phase 5 M17: the Project Brain — a persisted, structured understanding
//! of an opened project (stack, architecture, conventions, testing setup,
//! important files), generated from the project's own real repository
//! content, never fabricated. Two halves, deliberately kept separate so the
//! non-AI half is fully unit-testable without a live API key (this
//! environment has none in CI):
//!
//!   * [`gather_repo_context`]: pure file-scanning + `GitService::log` — no
//!     AI, no network. Reuses `project_detect`'s marker/convention-file
//!     scanning (M12) and `orchestrator::planner`'s shallow top-level
//!     directory listing (M8) rather than reimplementing either.
//!   * [`analyze_project`]: exactly one forced structured-output Anthropic
//!     call built from [`gather_repo_context`]'s output, using the same
//!     `AnthropicClient::request_structured_tool_call` one-shot pattern
//!     `orchestrator::planner::propose_plan` and `agent::reviewer`'s
//!     `request_review_submission` already established (M8/M14) — not a new
//!     calling convention.
//!
//! [`should_regenerate`] is the regeneration policy: a pure decision
//! function (no I/O) over the existing brain's `source_commit_sha` (if any)
//! and a window of the repository's most recent commit shas — directly
//! unit-testable. The plan explicitly calls for *not* regenerating on every
//! keystroke/commit, so this only says yes once `REGENERATE_AFTER_COMMITS`
//! commits have genuinely landed since the brain was last generated, or
//! when there's no brain yet, or when the caller passes `force: true` (an
//! explicit "Regenerate" button press — always honored).
//!
//! [`regenerate_brain`] is the real end-to-end entry point
//! `commands::brain_commands::regenerate_project_brain` calls for an
//! explicit user-requested refresh; [`maybe_auto_regenerate`] is the
//! fire-and-forget background check `commands::project_commands::
//! open_project`/`init_project` run on every open — the same
//! "never block the caller, swallow failures" pattern M14's reviewer
//! auto-trigger uses in `orchestrator::scheduler::apply_task_outcome`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::agent::anthropic_client::{AnthropicClient, MessageParam, StreamOutcome, ToolDefinition};
use crate::db::models::ProjectBrain;
use crate::db::repository::{model_configs as model_configs_repo, project_brain as project_brain_repo, repositories as repositories_repo};
use crate::error::{AppError, AppResult};
use crate::git::{CommitInfo, GitCliService, GitService};
use crate::orchestrator::planner::shallow_top_level_listing;
use crate::os_adapter;
use crate::project_detect;
use crate::secrets;
use crate::state::AppState;

const SUBMIT_PROJECT_BRAIN_TOOL: &str = "submit_project_brain";
const FALLBACK_MODEL_ID: &str = "claude-sonnet-5";
const FALLBACK_MAX_TOKENS: u32 = 4096;

/// How many of a README's leading lines are handed to the analysis prompt —
/// enough for a real description/overview section without hauling in an
/// entire (possibly huge) README.
const README_EXCERPT_LINES: usize = 40;

/// How many commits [`gather_repo_context`] pulls via `GitService::log` for
/// the prompt's "recent history" signal — a shallow window, same spirit as
/// `orchestrator::planner::build_repo_context`'s own 5-commit summary, just
/// slightly deeper since the Brain is meant to be a fuller analysis.
const RECENT_COMMITS_FOR_PROMPT: u32 = 20;

/// How far behind HEAD a brain's `source_commit_sha` can fall before
/// [`should_regenerate`] calls it stale. The plan explicitly calls for
/// *not* regenerating on every keystroke/commit; this is a simple,
/// documented "enough has genuinely changed" threshold rather than diffing
/// file contents on every project open.
pub const REGENERATE_AFTER_COMMITS: usize = 20;

// ---------------------------------------------------------------------
// Repo context gathering — pure file-scanning + git log, no AI, fully
// unit-testable (see the tests below).
// ---------------------------------------------------------------------

/// Real repository signals gathered for the analysis prompt. Every field
/// here is either a direct file-scan result or real git history — never a
/// guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoContext {
    pub folder_name: String,
    /// Marker files present (`package.json`, `Cargo.toml`, ...) — see
    /// `project_detect::scan_present_marker_files`. More than one can be
    /// present (a polyglot repo, e.g. this one); all are reported here,
    /// unlike `project_detect::detect_commands`'s single-ecosystem
    /// precedence.
    pub marker_files: Vec<String>,
    /// Convention/config files present (`tsconfig.json`, `.eslintrc*`, ...)
    /// — see `project_detect::scan_present_convention_files`.
    pub convention_files: Vec<String>,
    /// The detected test/lint/build commands, reused as-is from M12's
    /// `project_detect::detect_commands` rather than re-derived — a real
    /// testing-setup signal.
    pub test_command: Option<String>,
    pub lint_command: Option<String>,
    pub build_command: Option<String>,
    /// The first [`README_EXCERPT_LINES`] lines of `README.md`, if present.
    pub readme_excerpt: Option<String>,
    /// A shallow, non-recursive top-level directory listing — reused
    /// verbatim from `orchestrator::planner::shallow_top_level_listing`
    /// (M8), not a second implementation of the same scan.
    pub top_level_listing: String,
    /// The most recent commits, most recent first — reused directly from
    /// `GitService::log` (M2/M5/M15), not reimplemented here.
    pub recent_commits: Vec<CommitInfo>,
}

/// Gathers [`RepoContext`] for `repo_root` — no AI call, no network, fully
/// deterministic given the same files/git history, so this is unit-tested
/// directly against real tempdir fixtures (see the tests below) without any
/// live API key.
pub fn gather_repo_context(repo_root: &Path, git: &dyn GitService) -> RepoContext {
    let folder_name =
        repo_root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| repo_root.to_string_lossy().into_owned());

    let detected = project_detect::detect_commands(repo_root);

    RepoContext {
        folder_name,
        marker_files: project_detect::scan_present_marker_files(repo_root),
        convention_files: project_detect::scan_present_convention_files(repo_root),
        test_command: detected.test_command,
        lint_command: detected.lint_command,
        build_command: detected.build_command,
        readme_excerpt: read_readme_excerpt(repo_root),
        top_level_listing: shallow_top_level_listing(repo_root),
        recent_commits: git.log(repo_root, RECENT_COMMITS_FOR_PROMPT).unwrap_or_default(),
    }
}

/// The first [`README_EXCERPT_LINES`] lines of `root`'s README, if one
/// exists under any of the common casings — `None` (never a fabricated
/// summary) when no README is present or it's empty.
fn read_readme_excerpt(root: &Path) -> Option<String> {
    let candidates = ["README.md", "README.MD", "Readme.md", "readme.md"];
    let path = candidates.iter().map(|name| root.join(name)).find(|p| p.is_file())?;
    let contents = std::fs::read_to_string(path).ok()?;
    let excerpt: String = contents.lines().take(README_EXCERPT_LINES).collect::<Vec<_>>().join("\n");
    if excerpt.trim().is_empty() {
        None
    } else {
        Some(excerpt)
    }
}

/// Renders [`RepoContext`] into the one user message the analysis call
/// sends — grounds the model in exactly what was actually found, with every
/// absent signal spelled out honestly (`"(not detected)"`, `"(no README
/// found)"`, ...) rather than omitted, so the model never has to guess
/// whether something is missing or just wasn't mentioned.
fn build_analysis_prompt(context: &RepoContext) -> String {
    let folder_name = &context.folder_name;
    let marker_files = if context.marker_files.is_empty() { "(none found)".to_string() } else { context.marker_files.join(", ") };
    let convention_files =
        if context.convention_files.is_empty() { "(none found)".to_string() } else { context.convention_files.join(", ") };
    let test_command = context.test_command.as_deref().unwrap_or("(not detected)");
    let lint_command = context.lint_command.as_deref().unwrap_or("(not detected)");
    let build_command = context.build_command.as_deref().unwrap_or("(not detected)");
    let readme = context.readme_excerpt.as_deref().unwrap_or("(no README found)");
    let top_level_listing = &context.top_level_listing;
    let commits = if context.recent_commits.is_empty() {
        "(no commits yet)".to_string()
    } else {
        context.recent_commits.iter().map(|c| format!("- {} {}", c.short_sha, c.subject)).collect::<Vec<_>>().join("\n")
    };

    format!(
        "Analyze this real repository and produce a structured understanding of it. Only report what the \
         information below actually supports — never invent a technology, convention, testing tool, or file that \
         isn't evidenced here.\n\n\
         Repository folder: {folder_name}\n\n\
         Package/project marker files found at the root: {marker_files}\n\
         Convention/config files found at the root: {convention_files}\n\
         Detected commands — test: {test_command}, lint: {lint_command}, build: {build_command}\n\n\
         Top-level contents:\n{top_level_listing}\n\n\
         README excerpt:\n{readme}\n\n\
         Recent commits (most recent first):\n{commits}"
    )
}

// ---------------------------------------------------------------------
// The one structured-output analysis call.
// ---------------------------------------------------------------------

/// One important file/directory the model called out, with why it matters
/// — mirrors the plan's own example layout ("Important files: src/app,
/// src/api, src/components").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainImportantFile {
    pub path: String,
    pub why: String,
}

/// The `submit_project_brain` tool call's parsed input — the model's full
/// structured analysis before it's persisted as a [`ProjectBrain`] row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectBrainAnalysis {
    pub stack: Vec<String>,
    pub architecture: String,
    pub conventions: Vec<String>,
    pub testing: Vec<String>,
    #[serde(default)]
    pub important_files: Vec<BrainImportantFile>,
}

fn submit_project_brain_tool() -> ToolDefinition {
    ToolDefinition {
        name: SUBMIT_PROJECT_BRAIN_TOOL.to_string(),
        description: "Submit your finished structured understanding of this repository, grounded only in the \
                       real context you were given — never invent a technology, convention, testing tool, or file \
                       that wasn't actually evidenced."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "stack": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The real languages/frameworks/runtimes/datastores this project actually uses, e.g. [\"React\", \"TypeScript\", \"Rust\", \"SQLite\"]."
                },
                "architecture": {
                    "type": "string",
                    "description": "A short, structured description of the project's architecture — e.g. labeled lines like 'Frontend -> React / Backend -> Rust (Tauri) / Database -> SQLite'."
                },
                "conventions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Real coding conventions evidenced by the repo's own config files, e.g. [\"Strict TypeScript\", \"ESLint\", \"Prettier\"]."
                },
                "testing": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The real testing setup evidenced by the repo, e.g. [\"Vitest\", \"Playwright\", \"cargo test\"]."
                },
                "important_files": {
                    "type": "array",
                    "description": "The handful of files/directories most worth knowing about, each with a short reason.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "why": { "type": "string", "description": "Why this file/directory matters." }
                        },
                        "required": ["path", "why"]
                    }
                }
            },
            "required": ["stack", "architecture", "conventions", "testing"]
        }),
    }
}

fn system_prompt() -> &'static str {
    "You are a meticulous software architect analyzing a real codebase. You will be given real signals gathered \
     directly from the repository itself (its own marker/config files, detected commands, README, top-level \
     listing, and recent commit history) — never invent a technology, convention, testing tool, or important file \
     that isn't actually evidenced by what you're given. If a category has no real evidence, return an honest \
     empty list rather than guessing. Call `submit_project_brain` exactly once with your finished structured \
     analysis."
}

/// Parses a `submit_project_brain` tool call's `input` JSON into a
/// [`ProjectBrainAnalysis`]. Split out from the API-calling code so it's
/// fully unit-testable against hand-built fixtures, per this milestone's
/// constraints (no live API key in CI) — same split `orchestrator::planner::
/// parse_mission_plan` and `agent::reviewer::parse_review_submission` use.
pub fn parse_project_brain_analysis(input: &serde_json::Value) -> AppResult<ProjectBrainAnalysis> {
    serde_json::from_value(input.clone())
        .map_err(|e| AppError::Other(format!("model returned a malformed project brain analysis: {e}")))
}

/// Runs the one Anthropic call that turns `repo_root`'s real content into a
/// [`ProjectBrainAnalysis`] — the same `request_structured_tool_call`
/// one-shot pattern `orchestrator::planner::propose_plan` already
/// established. Returns `Err` for any genuine failure (API error, the model
/// not calling `submit_project_brain`, a malformed response) — the caller
/// ([`regenerate_brain`]) is responsible for surfacing that honestly rather
/// than persisting a fabricated brain.
pub async fn analyze_project(
    client: &AnthropicClient,
    model: &str,
    max_tokens: u32,
    repo_root: &Path,
    git: &dyn GitService,
    cancel: &CancellationToken,
) -> AppResult<ProjectBrainAnalysis> {
    let context = gather_repo_context(repo_root, git);
    let prompt = build_analysis_prompt(&context);
    let messages = vec![MessageParam::user_text(prompt)];
    let tool = submit_project_brain_tool();

    let outcome = client.request_structured_tool_call(model, max_tokens, system_prompt(), &messages, &tool, cancel).await?;

    let turn = match outcome {
        StreamOutcome::Turn(turn) => turn,
        StreamOutcome::Cancelled => return Err(AppError::Other("project brain analysis was cancelled".to_string())),
    };

    let analysis_input = turn
        .tool_uses()
        .find(|(_, name, _)| *name == SUBMIT_PROJECT_BRAIN_TOOL)
        .map(|(_, _, input)| input)
        .ok_or_else(|| {
            let text = turn.text();
            if text.trim().is_empty() {
                AppError::Other(format!("the model did not call `{SUBMIT_PROJECT_BRAIN_TOOL}`"))
            } else {
                AppError::Other(format!("the model did not call `{SUBMIT_PROJECT_BRAIN_TOOL}` — it said: {text}"))
            }
        })?;

    parse_project_brain_analysis(analysis_input)
}

// ---------------------------------------------------------------------
// Regeneration policy — pure, no I/O, fully unit-testable.
// ---------------------------------------------------------------------

/// Decides whether a Project Brain should be regenerated right now.
/// `existing_source_sha` is the currently-stored brain's `source_commit_sha`
/// (`None` if one has never been generated). `recent_commit_shas` is HEAD's
/// own commit history, most-recent-first (the real caller passes
/// `GitService::log`'s own shas, capped to a small window) — just enough to
/// tell whether `existing_source_sha` is still within the last
/// [`REGENERATE_AFTER_COMMITS`] commits, without walking all of history.
///
/// Three real reasons to say yes: (1) no brain exists yet, (2) the caller
/// passed `force: true` (an explicit "Regenerate" button press — always
/// honored, regardless of staleness), or (3) `existing_source_sha` is at
/// least [`REGENERATE_AFTER_COMMITS`] commits behind HEAD, or wasn't found
/// in the fetched window at all (either genuinely stale, or history was
/// rewritten out from under it — either way, honest to treat as stale). The
/// plan explicitly calls for *not* regenerating on every keystroke/commit,
/// so this deliberately never says yes just because *any* new commit
/// landed.
pub fn should_regenerate(existing_source_sha: Option<&str>, recent_commit_shas: &[String], force: bool) -> bool {
    if force {
        return true;
    }
    let Some(existing_sha) = existing_source_sha else {
        return true;
    };
    match recent_commit_shas.iter().position(|sha| sha == existing_sha) {
        Some(distance_from_head) => distance_from_head >= REGENERATE_AFTER_COMMITS,
        None => true,
    }
}

// ---------------------------------------------------------------------
// The real entry points.
// ---------------------------------------------------------------------

/// Runs a real Project Brain generation end to end for `project_id`: loads
/// its repository root (failing with a real error if the project has none —
/// never fabricates a brain for a project without one), confirms an
/// Anthropic API key is configured (the same honest failure every other
/// AI-calling milestone uses — never a fake analysis), runs
/// [`analyze_project`], and persists the result via
/// `db::repository::project_brain::upsert` keyed to the repository's real
/// current HEAD commit (empty string if the repository has no commits yet).
/// Always regenerates — the caller
/// (`commands::brain_commands::regenerate_project_brain`, or
/// [`maybe_auto_regenerate`] once its own policy check says yes) decides
/// *whether* to call this at all.
pub async fn regenerate_brain(app: &AppHandle, project_id: &str) -> AppResult<ProjectBrain> {
    let (repo_root, model_id, max_tokens) = {
        let app = app.clone();
        let project_id = project_id.to_string();
        tauri::async_runtime::spawn_blocking(move || -> AppResult<(PathBuf, String, u32)> {
            let state = app.state::<AppState>();
            let conn = state.db.get()?;
            let repository = repositories_repo::get_by_project_id(&conn, &project_id)?
                .ok_or_else(|| AppError::NotFound(format!("no repository registered for project {project_id}")))?;
            let default_model = model_configs_repo::get_default(&conn)?;
            let model_id = default_model.as_ref().map(|m| m.model_id.clone()).unwrap_or_else(|| FALLBACK_MODEL_ID.to_string());
            let max_tokens = default_model.map(|m| m.max_output_tokens as u32).unwrap_or(FALLBACK_MAX_TOKENS);
            Ok((PathBuf::from(repository.root_path), model_id, max_tokens))
        })
        .await
        .map_err(|e| AppError::Other(format!("background task failed: {e}")))??
    };

    // Same "fail loudly, never silently fabricate a result" pattern
    // `agent::tool_loop`/`orchestrator::planner`/`agent::reviewer` all use
    // before their own first model call.
    let api_key = tauri::async_runtime::spawn_blocking(|| secrets::get_secret(secrets::ANTHROPIC_API_KEY))
        .await
        .map_err(|e| AppError::Other(format!("API key lookup panicked: {e}")))??;
    let Some(api_key) = api_key else {
        return Err(AppError::InvalidInput(
            "No Anthropic API key is configured. Add one in Settings, then try again.".to_string(),
        ));
    };

    let client = AnthropicClient::new(api_key)?;
    let os_adapter = os_adapter::current();
    let git_service: Box<dyn GitService> = Box::new(GitCliService::new(os_adapter.as_ref()));
    // No live progress to cancel mid-flight for a single structured-output
    // call — a fresh, never-fired token is enough to satisfy
    // `request_structured_tool_call`'s signature, same as
    // `mission_commands::create_mission`.
    let cancel = CancellationToken::new();

    let analysis = analyze_project(&client, &model_id, max_tokens, &repo_root, git_service.as_ref(), &cancel).await?;

    let source_commit_sha =
        git_service.log(&repo_root, 1).ok().and_then(|commits| commits.into_iter().next()).map(|c| c.sha).unwrap_or_default();

    let stack_json = serde_json::to_string(&analysis.stack).map_err(|e| AppError::Other(format!("failed to serialize stack: {e}")))?;
    let architecture_json =
        serde_json::to_string(&analysis.architecture).map_err(|e| AppError::Other(format!("failed to serialize architecture: {e}")))?;
    let conventions_json =
        serde_json::to_string(&analysis.conventions).map_err(|e| AppError::Other(format!("failed to serialize conventions: {e}")))?;
    let testing_json = serde_json::to_string(&analysis.testing).map_err(|e| AppError::Other(format!("failed to serialize testing: {e}")))?;
    let important_files_json = serde_json::to_string(&analysis.important_files)
        .map_err(|e| AppError::Other(format!("failed to serialize important_files: {e}")))?;

    let app = app.clone();
    let project_id = project_id.to_string();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<ProjectBrain> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        project_brain_repo::upsert(
            &conn,
            &project_id,
            &stack_json,
            &architecture_json,
            &conventions_json,
            &testing_json,
            &important_files_json,
            &source_commit_sha,
        )
    })
    .await
    .map_err(|e| AppError::Other(format!("background task failed: {e}")))?
}

/// The fire-and-forget auto-regeneration check `commands::project_commands::
/// open_project`/`init_project` run after every successful open — same
/// pattern as M14's reviewer auto-trigger (`orchestrator::scheduler`'s
/// `tauri::async_runtime::spawn` right after a task reaches `done`): never
/// blocks the caller, and any failure (no API key configured, an API error,
/// no repository registered) is swallowed rather than surfaced, since this
/// is a background refresh the user didn't explicitly ask for in this call.
/// An explicit "Regenerate" button press goes through
/// [`regenerate_brain`]/`commands::brain_commands::regenerate_project_brain`
/// instead, where failures *do* surface.
pub fn maybe_auto_regenerate(app: &AppHandle, project_id: &str) {
    let app = app.clone();
    let project_id = project_id.to_string();
    tauri::async_runtime::spawn(async move {
        let check_app = app.clone();
        let check_project_id = project_id.clone();
        let should = tauri::async_runtime::spawn_blocking(move || -> AppResult<bool> {
            let state = check_app.state::<AppState>();
            let conn = state.db.get()?;
            let Some(repository) = repositories_repo::get_by_project_id(&conn, &check_project_id)? else {
                return Ok(false);
            };
            let existing_sha = project_brain_repo::get_by_project_id(&conn, &check_project_id)?.map(|b| b.source_commit_sha);
            let os_adapter = os_adapter::current();
            let git_service = GitCliService::new(os_adapter.as_ref());
            let recent_shas: Vec<String> = git_service
                .log(Path::new(&repository.root_path), (REGENERATE_AFTER_COMMITS + 1) as u32)
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.sha)
                .collect();
            Ok(should_regenerate(existing_sha.as_deref(), &recent_shas, false))
        })
        .await;

        if matches!(should, Ok(Ok(true))) {
            let _ = regenerate_brain(&app, &project_id).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // -- should_regenerate (pure) --------------------------------------

    fn shas(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("sha{i}")).collect()
    }

    #[test]
    fn regenerates_when_no_brain_exists_yet() {
        assert!(should_regenerate(None, &shas(5), false));
    }

    #[test]
    fn regenerates_when_no_brain_exists_yet_even_with_no_commit_history() {
        assert!(should_regenerate(None, &[], false));
    }

    #[test]
    fn force_always_regenerates_even_when_fresh() {
        let recent = shas(5);
        assert!(should_regenerate(Some("sha0"), &recent, true));
    }

    #[test]
    fn does_not_regenerate_when_source_sha_is_still_head() {
        let recent = shas(5);
        assert!(!should_regenerate(Some("sha0"), &recent, false));
    }

    #[test]
    fn does_not_regenerate_when_within_the_threshold() {
        let recent = shas(REGENERATE_AFTER_COMMITS);
        let stale_but_within_window = format!("sha{}", REGENERATE_AFTER_COMMITS - 1);
        assert!(!should_regenerate(Some(&stale_but_within_window), &recent, false));
    }

    #[test]
    fn regenerates_once_at_least_the_threshold_of_commits_have_landed() {
        let recent = shas(REGENERATE_AFTER_COMMITS + 1);
        let exactly_at_threshold = format!("sha{REGENERATE_AFTER_COMMITS}");
        assert!(should_regenerate(Some(&exactly_at_threshold), &recent, false));
    }

    #[test]
    fn regenerates_when_source_sha_is_not_found_in_the_window_at_all() {
        let recent = shas(5);
        assert!(should_regenerate(Some("sha-does-not-exist-anymore"), &recent, false));
    }

    // -- parse_project_brain_analysis (pure) ---------------------------

    #[test]
    fn parses_a_well_formed_analysis_with_important_files() {
        let input = json!({
            "stack": ["React", "TypeScript", "Rust"],
            "architecture": "Frontend -> React / Backend -> Rust (Tauri) / Database -> SQLite",
            "conventions": ["Strict TypeScript", "ESLint"],
            "testing": ["Vitest", "cargo test"],
            "important_files": [
                { "path": "src-tauri/src/lib.rs", "why": "Command registration entry point" }
            ]
        });
        let analysis = parse_project_brain_analysis(&input).expect("should parse");
        assert_eq!(analysis.stack, vec!["React".to_string(), "TypeScript".to_string(), "Rust".to_string()]);
        assert_eq!(analysis.important_files.len(), 1);
        assert_eq!(analysis.important_files[0].path, "src-tauri/src/lib.rs");
    }

    #[test]
    fn missing_important_files_defaults_to_empty_rather_than_erroring() {
        let input = json!({
            "stack": ["Python"],
            "architecture": "A script",
            "conventions": [],
            "testing": ["pytest"]
        });
        let analysis = parse_project_brain_analysis(&input).expect("should parse");
        assert!(analysis.important_files.is_empty());
    }

    #[test]
    fn empty_arrays_parse_as_an_honest_empty_analysis_rather_than_erroring() {
        let input = json!({ "stack": [], "architecture": "Unknown", "conventions": [], "testing": [] });
        let analysis = parse_project_brain_analysis(&input).expect("an empty-but-honest analysis is still valid");
        assert!(analysis.stack.is_empty());
        assert!(analysis.testing.is_empty());
    }

    #[test]
    fn missing_required_field_fails_clearly() {
        let input = json!({ "stack": ["Go"], "conventions": [], "testing": [] });
        let err = parse_project_brain_analysis(&input).expect_err("missing architecture should fail to parse");
        assert!(err.to_string().contains("malformed project brain analysis"));
    }

    // -- build_analysis_prompt (pure) ----------------------------------

    fn empty_context() -> RepoContext {
        RepoContext {
            folder_name: "demo".to_string(),
            marker_files: vec![],
            convention_files: vec![],
            test_command: None,
            lint_command: None,
            build_command: None,
            readme_excerpt: None,
            top_level_listing: "(empty)".to_string(),
            recent_commits: vec![],
        }
    }

    #[test]
    fn prompt_honestly_labels_every_absent_signal_rather_than_omitting_it() {
        let prompt = build_analysis_prompt(&empty_context());
        assert!(prompt.contains("(none found)"));
        assert!(prompt.contains("(not detected)"));
        assert!(prompt.contains("(no README found)"));
        assert!(prompt.contains("(no commits yet)"));
        assert!(prompt.contains("demo"));
    }

    // -- gather_repo_context (real filesystem + real git, no AI) ------

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git").args(args).current_dir(repo).status().expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_git_repo(dir: &Path) {
        git(dir, &["init"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "user.name", "Test"]);
    }

    fn commit_all(dir: &Path, message: &str) {
        git(dir, &["add", "."]);
        git(dir, &["commit", "-m", message]);
    }

    fn real_git_service() -> GitCliService {
        GitCliService::new(os_adapter::current().as_ref())
    }

    #[test]
    fn gathers_real_signals_from_a_node_project_with_conventions_and_git_history() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"scripts":{"test":"vitest"}}"#).unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Demo Project\n\nA demo project for tests.\n").unwrap();
        init_git_repo(dir.path());
        commit_all(dir.path(), "init");

        let git_service = real_git_service();
        let context = gather_repo_context(dir.path(), &git_service);

        assert!(context.marker_files.contains(&"package.json".to_string()));
        assert!(context.convention_files.contains(&"tsconfig.json".to_string()));
        assert_eq!(context.test_command.as_deref(), Some("npm test"));
        assert!(context.readme_excerpt.as_deref().unwrap().contains("Demo Project"));
        assert_eq!(context.recent_commits.len(), 1);
        assert_eq!(context.recent_commits[0].subject, "init");
    }

    #[test]
    fn missing_readme_and_markers_are_honestly_absent_rather_than_fabricated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitkeep"), "").unwrap();
        init_git_repo(dir.path());
        commit_all(dir.path(), "init without a readme");

        let git_service = real_git_service();
        let context = gather_repo_context(dir.path(), &git_service);

        assert!(context.readme_excerpt.is_none());
        assert!(context.marker_files.is_empty());
        assert!(context.convention_files.is_empty());
        assert!(context.test_command.is_none());
        assert_eq!(context.recent_commits.len(), 1);
    }

    #[test]
    fn a_directory_with_no_git_history_at_all_reports_an_honest_empty_commit_log() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        // Deliberately never `git init`-ed.

        let git_service = real_git_service();
        let context = gather_repo_context(dir.path(), &git_service);

        assert!(context.recent_commits.is_empty());
        assert!(context.marker_files.contains(&"package.json".to_string()));
    }
}

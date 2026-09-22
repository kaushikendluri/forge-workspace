//! Dispatch for the agent-facing tool set (schemas in `agent::schema`).
//!
//! Every function here returns a normal, model-facing result (success text,
//! or an error string meant to come back as a `tool_result` with
//! `is_error: true`) rather than a Rust-level `Err` — a missing file, an
//! ambiguous `edit_file` match, a failing shell command, or an unconfigured
//! `run_tests` command are all *expected, recoverable* outcomes the model
//! should see and can react to, not reasons to abort the whole run. The one
//! exception is `report_completion`, which deliberately ends the run instead
//! of producing a normal tool result.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::browser::BrowserManager;
use crate::db::models::{Task, TestRunKind, TestRunStatus, VisualSnapshotKind};
use crate::db::repository::{
    agent_messages as agent_messages_repo, tasks as tasks_repo, test_runs as test_runs_repo,
    visual_snapshots as visual_snapshots_repo,
};
use crate::db::DbPool;
use crate::git::GitService;
use crate::os_adapter::OperatingSystemAdapter;

use super::path_guard::resolve_in_workspace;
use super::process;

/// M11: set on `ToolContext` only when this run is executing as part of a
/// mission (the scheduler started it for a specific mission task, via
/// `orchestrator::scheduler::start_task_execution`) — see
/// `agent::tool_loop::run_agent_loop_inner`, which resolves this once via
/// `tasks_repo::get_by_agent_run_id`. `None` for a solo run started directly
/// from the Agents page. `send_message` is the only tool that reads this.
#[derive(Debug, Clone)]
pub struct MissionContext {
    pub mission_id: String,
    pub task_id: String,
}

/// Everything a tool call needs that isn't in its own JSON `input` — the
/// run's workspace root plus the services/config it's allowed to touch.
/// Deliberately holds only `workspace_root` (a specific run's worktree), not
/// the primary repository root: every filesystem tool is scoped to this one
/// directory, never the checkout the user is actively working in.
pub struct ToolContext<'a> {
    pub workspace_root: PathBuf,
    pub git_service: &'a dyn GitService,
    pub os_adapter: &'a dyn OperatingSystemAdapter,
    pub test_command: Option<String>,
    pub lint_command: Option<String>,
    pub build_command: Option<String>,
    pub tool_timeout: Duration,
    pub cancel: CancellationToken,
    /// M11: this run's own id (`agent_runs.id`) — none of the tools before
    /// M11 needed to know their own run's id (they only ever touch the
    /// workspace filesystem/git/shell); `send_message` needs it as the new
    /// message's `from_agent_run_id`.
    pub agent_run_id: String,
    /// M11: `Some` only for a mission-context run — see [`MissionContext`].
    pub mission_context: Option<MissionContext>,
    /// M11: pooled DB access for `send_message` to look up the mission's
    /// other tasks (to resolve `to_task_title`) and persist the
    /// `agent_messages` row. M13 also uses it (via `test_runs_repo`) so an
    /// agent's `run_tests` call persists a real `test_runs` row the same way
    /// a manual Testing-tab run does. Every other tool here is DB-free.
    pub db_pool: DbPool,
    /// M13: the run's owning project — needed only to record `run_tests`
    /// calls into `test_runs` (a project-scoped table, like the Testing
    /// tab's manual runs).
    pub project_id: String,
    /// M16: this run's lazily-launched browser session registry — see
    /// `browser::BrowserManager`'s own docs. Shared (an `Arc`) rather than
    /// borrowed since it outlives any single `ToolContext`/tool call and is
    /// also needed by `agent::tool_loop::finish_run`'s cleanup, independent
    /// of whatever `ToolContext` a given tool call happened to build.
    pub browser_manager: Arc<BrowserManager>,
    /// M16: where `browser_screenshot` saves real PNG files — see
    /// `AppState::screenshots_dir`'s own docs.
    pub screenshots_dir: PathBuf,
}

/// What running one tool call produced.
pub enum ToolRunOutcome {
    /// A normal `tool_result` to send back to the model.
    Result {
        output: String,
        is_error: bool,
        /// M13: the `test_runs` row id this call persisted, if it was a
        /// `run_tests` call — `None` for every other tool, and also `None`
        /// for `run_tests` if the DB write itself failed (best-effort: a DB
        /// hiccup here must never block the actual test run).
        test_run_id: Option<String>,
    },
    /// `report_completion` was called — ends the run instead of looping.
    Completion { summary: String, success: bool },
}

fn ok(output: impl Into<String>) -> ToolRunOutcome {
    ToolRunOutcome::Result { output: output.into(), is_error: false, test_run_id: None }
}

fn err(output: impl Into<String>) -> ToolRunOutcome {
    ToolRunOutcome::Result { output: output.into(), is_error: true, test_run_id: None }
}

fn require_str<'a>(input: &'a Value, field: &str) -> Result<&'a str, String> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or non-string required field '{field}'"))
}

/// Dispatches one `tool_use` block by name. Unknown tool names come back as
/// a normal `tool_result` error (the model asked for something that doesn't
/// exist) rather than a Rust error, for the same reason every other tool
/// failure here is a `tool_result` — it's something the model can see and
/// correct.
pub async fn dispatch_tool(ctx: &ToolContext<'_>, name: &str, input: &Value) -> ToolRunOutcome {
    match name {
        "read_file" => read_file(ctx, input),
        "write_file" => write_file(ctx, input),
        "edit_file" => edit_file(ctx, input),
        "list_directory" => list_directory(ctx, input),
        "search_files" => search_files(ctx, input),
        "search_code" => search_code(ctx, input),
        "run_command" => run_command_tool(ctx, input).await,
        "run_tests" => run_tests_tool(ctx).await,
        "run_linter" => run_configured_command(ctx, ctx.lint_command.clone(), "lint").await,
        "run_build" => run_configured_command(ctx, ctx.build_command.clone(), "build").await,
        "git_status" => git_status(ctx),
        "git_diff" => git_diff(ctx, input),
        "git_log" => git_log(ctx, input),
        "send_message" => send_message(ctx, input),
        "browser_open" => browser_open(ctx, input).await,
        "browser_click" => browser_click(ctx, input).await,
        "browser_type" => browser_type_tool(ctx, input).await,
        "browser_screenshot" => browser_screenshot_tool(ctx, input).await,
        "report_completion" => report_completion(input),
        other => err(format!("unknown tool '{other}'")),
    }
}

// ---------------------------------------------------------------------
// Filesystem tools — every path goes through `resolve_in_workspace` first.
// ---------------------------------------------------------------------

fn read_file(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let path = match require_str(input, "path") {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let resolved = match resolve_in_workspace(&ctx.workspace_root, path) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    match std::fs::read(&resolved) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => ok(text),
            Err(_) => err(format!("'{path}' is not valid UTF-8 text (looks like a binary file)")),
        },
        Err(e) => err(format!("failed to read '{path}': {e}")),
    }
}

fn write_file(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let path = match require_str(input, "path") {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let content = match require_str(input, "content") {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    let resolved = match resolve_in_workspace(&ctx.workspace_root, path) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    if let Some(parent) = resolved.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return err(format!("failed to create parent directories for '{path}': {e}"));
        }
    }
    match std::fs::write(&resolved, content) {
        Ok(()) => ok(format!("wrote {} bytes to '{path}'", content.len())),
        Err(e) => err(format!("failed to write '{path}': {e}")),
    }
}

fn edit_file(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let path = match require_str(input, "path") {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let old_string = match require_str(input, "old_string") {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let new_string = match require_str(input, "new_string") {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    if old_string.is_empty() {
        return err("old_string must not be empty".to_string());
    }
    let resolved = match resolve_in_workspace(&ctx.workspace_root, path) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let contents = match std::fs::read_to_string(&resolved) {
        Ok(c) => c,
        Err(e) => return err(format!("failed to read '{path}': {e}")),
    };

    let occurrences = contents.matches(old_string).count();
    if occurrences == 0 {
        return err(format!("old_string was not found in '{path}'"));
    }
    if occurrences > 1 {
        return err(format!(
            "old_string occurs {occurrences} times in '{path}' — it must be unique; include more \
             surrounding context to disambiguate"
        ));
    }

    let updated = contents.replacen(old_string, new_string, 1);
    match std::fs::write(&resolved, updated) {
        Ok(()) => ok(format!("edited '{path}'")),
        Err(e) => err(format!("failed to write '{path}': {e}")),
    }
}

fn list_directory(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let path = input.get("path").and_then(Value::as_str).unwrap_or(".");
    let resolved = match resolve_in_workspace(&ctx.workspace_root, path) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let read_dir = match std::fs::read_dir(&resolved) {
        Ok(rd) => rd,
        Err(e) => return err(format!("failed to list '{path}': {e}")),
    };

    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
    }
    entries.sort();

    if entries.is_empty() {
        ok(format!("'{path}' is empty"))
    } else {
        ok(entries.join("\n"))
    }
}

const SKIPPED_DIR_NAMES: [&str; 6] = [".git", "node_modules", "target", "dist", "build", ".forge-workspace"];
const MAX_WALK_ENTRIES: usize = 20_000;
const MAX_SEARCH_RESULTS: usize = 200;

/// Recursively collects every file's path relative to `root` (forward-slash
/// separated regardless of host OS, so glob patterns and displayed results
/// are consistent across platforms), skipping VCS/build/dependency
/// directories and capping at `MAX_WALK_ENTRIES` files so a huge repository
/// can't make a single tool call run forever.
fn walk_files(root: &Path) -> (Vec<String>, bool) {
    let mut results = Vec::new();
    let mut truncated = false;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else { continue };
        for entry in read_dir.flatten() {
            if results.len() >= MAX_WALK_ENTRIES {
                truncated = true;
                return (results, truncated);
            }
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if SKIPPED_DIR_NAMES.contains(&name_str.as_ref()) {
                    continue;
                }
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                results.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    (results, truncated)
}

/// Translates a glob pattern (`*`, `**`, `?`) into an anchored regex.
/// `**` matches any sequence including `/`; `*` matches any sequence
/// excluding `/`; `?` matches one character excluding `/`. Everything else
/// is regex-escaped literally.
fn glob_to_regex(pattern: &str) -> String {
    let mut regex = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if chars.get(i + 1) == Some(&'*') {
                    regex.push_str(".*");
                    i += 2;
                } else {
                    regex.push_str("[^/]*");
                    i += 1;
                }
            }
            '?' => {
                regex.push_str("[^/]");
                i += 1;
            }
            c => {
                if "\\.+^$()[]{}|".contains(c) {
                    regex.push('\\');
                }
                regex.push(c);
                i += 1;
            }
        }
    }
    regex.push('$');
    regex
}

fn search_files(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let pattern = match require_str(input, "pattern") {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let regex_str = glob_to_regex(pattern);
    let regex = match regex::Regex::new(&regex_str) {
        Ok(r) => r,
        Err(e) => return err(format!("invalid glob pattern '{pattern}': {e}")),
    };

    let (all_files, truncated_walk) = walk_files(&ctx.workspace_root);
    let has_slash = pattern.contains('/');

    let mut matches: Vec<&String> = all_files
        .iter()
        .filter(|rel| {
            if has_slash {
                regex.is_match(rel)
            } else {
                let basename = rel.rsplit('/').next().unwrap_or(rel);
                regex.is_match(basename)
            }
        })
        .collect();
    matches.sort();

    let mut truncated = truncated_walk;
    if matches.len() > MAX_SEARCH_RESULTS {
        matches.truncate(MAX_SEARCH_RESULTS);
        truncated = true;
    }

    if matches.is_empty() {
        return ok(format!("no files matched '{pattern}'"));
    }
    let mut output = matches.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
    if truncated {
        output.push_str("\n… (results truncated)");
    }
    ok(output)
}

fn search_code(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let query = match require_str(input, "query") {
        Ok(q) => q,
        Err(e) => return err(e),
    };
    let use_regex = input.get("regex").and_then(Value::as_bool).unwrap_or(false);

    let regex = if use_regex {
        match regex::Regex::new(query) {
            Ok(r) => Some(r),
            Err(e) => return err(format!("invalid regex '{query}': {e}")),
        }
    } else {
        None
    };

    let (all_files, _truncated_walk) = walk_files(&ctx.workspace_root);
    let mut matches = Vec::new();
    'files: for rel in &all_files {
        let full = ctx.workspace_root.join(rel);
        let Ok(bytes) = std::fs::read(&full) else { continue };
        if bytes.len() > 5_000_000 || bytes.contains(&0) {
            continue; // skip large/binary files
        }
        let Ok(text) = String::from_utf8(bytes) else { continue };
        for (line_no, line) in text.lines().enumerate() {
            let is_match = match &regex {
                Some(re) => re.is_match(line),
                None => line.contains(query),
            };
            if is_match {
                matches.push(format!("{rel}:{}: {}", line_no + 1, line.trim()));
                if matches.len() >= MAX_SEARCH_RESULTS {
                    break 'files;
                }
            }
        }
    }

    if matches.is_empty() {
        return ok(format!("no matches for '{query}'"));
    }
    let mut output = matches.join("\n");
    if matches.len() >= MAX_SEARCH_RESULTS {
        output.push_str("\n… (results truncated)");
    }
    ok(output)
}

// ---------------------------------------------------------------------
// Shell execution — the actual spawn/timeout/cancel dance lives in
// `agent::process` (shared with M12's `commands::testing_commands`); this
// just formats that outcome into a `ToolRunOutcome` the model sees.
// ---------------------------------------------------------------------

async fn run_shell(ctx: &ToolContext<'_>, command: &str) -> ToolRunOutcome {
    let (text, is_error, _exit_code) = run_shell_text(ctx, command).await;
    if is_error { err(text) } else { ok(text) }
}

/// Same as `run_shell`, but returns the formatted output as a plain
/// `(text, is_error, exit_code)` tuple instead of a `ToolRunOutcome` —
/// `run_tests` needs the raw exit code to record a proper
/// `test_runs.exit_code` (M13), and building its own `ToolRunOutcome::Result`
/// (to also carry `test_run_id`) is far simpler from three plain values than
/// from trying to pick apart an already-constructed `ToolRunOutcome`.
async fn run_shell_text(ctx: &ToolContext<'_>, command: &str) -> (String, bool, Option<i64>) {
    let outcome = process::run_shell_command(ctx.os_adapter, &ctx.workspace_root, command, ctx.tool_timeout, ctx.cancel.clone()).await;
    match outcome {
        process::ProcessOutcome::Cancelled => {
            // `run_shell_command` owns the `Child` until it returns; by the
            // time this branch runs, `kill_on_drop(true)` has already killed
            // the real OS process rather than abandoning it.
            ("command cancelled: the agent run was stopped".to_string(), true, None)
        }
        process::ProcessOutcome::TimedOut { timeout_ms } => {
            (format!("command timed out after {timeout_ms}ms and was killed"), true, None)
        }
        process::ProcessOutcome::SpawnFailed(message) => (message, true, None),
        process::ProcessOutcome::Finished { stdout, stderr, exit_code, success } => {
            let mut text = format!(
                "exit code: {}\n",
                exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown (terminated by signal)".to_string())
            );
            if !stdout.is_empty() {
                text.push_str(&format!("stdout:\n{stdout}\n"));
            }
            if !stderr.is_empty() {
                text.push_str(&format!("stderr:\n{stderr}\n"));
            }
            (text, !success, exit_code.map(i64::from))
        }
    }
}

async fn run_command_tool(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let command = match require_str(input, "command") {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    run_shell(ctx, command).await
}

async fn run_configured_command(ctx: &ToolContext<'_>, configured: Option<String>, kind: &str) -> ToolRunOutcome {
    match configured {
        Some(command) if !command.trim().is_empty() => run_shell(ctx, &command).await,
        _ => err(format!(
            "no {kind} command configured for this project — set it in the project's Testing tab (or the \
             matching `project.<id>.{kind}_command` setting) first"
        )),
    }
}

/// `run_tests` specifically (not `run_linter`/`run_build`, out of M13's
/// scope): runs the project's configured test command exactly like
/// `run_configured_command` would, but also records it as a real
/// `test_runs` row — the same table/history `commands::testing_commands`'s
/// manual "Run tests" button already writes to — so `agent::tool_loop`'s
/// M13 `test_fix_cycle` activity events can reference an actual persisted
/// test run rather than duplicating its output. Recording is best-effort:
/// a DB failure here is logged into the returned `test_run_id` as `None`
/// and never blocks the actual test command from running or its result
/// from reaching the model.
async fn run_tests_tool(ctx: &ToolContext<'_>) -> ToolRunOutcome {
    let Some(command) = ctx.test_command.clone().filter(|c| !c.trim().is_empty()) else {
        return err(
            "no test command configured for this project — set it in the project's Testing tab (or the \
             matching `project.<id>.test_command` setting) first"
                .to_string(),
        );
    };

    let running_id = match ctx.db_pool.get() {
        Ok(conn) => test_runs_repo::insert_running(&conn, &ctx.project_id, TestRunKind::Test, &command).ok().map(|r| r.id),
        Err(_) => None,
    };

    let (output, is_error, exit_code) = run_shell_text(ctx, &command).await;

    if let Some(id) = &running_id {
        if let Ok(conn) = ctx.db_pool.get() {
            let status = if is_error { TestRunStatus::Failure } else { TestRunStatus::Success };
            let _ = test_runs_repo::complete(&conn, id, status, &output, exit_code);
        }
    }

    ToolRunOutcome::Result { output, is_error, test_run_id: running_id }
}

// ---------------------------------------------------------------------
// Git tools — reuse `GitService`; never reimplement git parsing here.
// ---------------------------------------------------------------------

fn git_status(ctx: &ToolContext<'_>) -> ToolRunOutcome {
    match ctx.git_service.status(&ctx.workspace_root) {
        Ok(status) => {
            let mut lines = vec![format!("branch: {}", status.current_branch.as_deref().unwrap_or("(detached)"))];
            for (label, entries) in [("staged", &status.staged), ("unstaged", &status.unstaged)] {
                if entries.is_empty() {
                    continue;
                }
                lines.push(format!("{label}:"));
                for entry in entries {
                    lines.push(format!("  {} {}", entry.status_code, entry.path));
                }
            }
            if !status.untracked.is_empty() {
                lines.push("untracked:".to_string());
                for path in &status.untracked {
                    lines.push(format!("  {path}"));
                }
            }
            ok(lines.join("\n"))
        }
        Err(e) => err(format!("git status failed: {e}")),
    }
}

fn git_diff(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let path = match require_str(input, "path") {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    // Validate the path the same way every filesystem tool does before
    // handing it to `GitService::diff_file`, which otherwise joins it onto
    // the workspace root unchecked (`repo_root.join(path)`) — without this,
    // `git_diff` would be a path-traversal hole distinct from (and just as
    // real as) the filesystem tools'.
    if let Err(e) = resolve_in_workspace(&ctx.workspace_root, path) {
        return err(e);
    }
    let normalized = path.replace('\\', "/");
    match ctx.git_service.diff_file(&ctx.workspace_root, &normalized) {
        Ok(diff) => {
            if diff.is_new_file {
                ok(format!("'{path}' is new (untracked/added):\n{}", diff.modified))
            } else if diff.is_deleted {
                ok(format!("'{path}' was deleted. Previous content:\n{}", diff.original))
            } else {
                ok(format!(
                    "--- {path} (HEAD) ---\n{}\n--- {path} (working tree) ---\n{}",
                    diff.original, diff.modified
                ))
            }
        }
        Err(e) => err(format!("git diff failed for '{path}': {e}")),
    }
}

fn git_log(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 200) as u32;
    match ctx.git_service.log(&ctx.workspace_root, limit) {
        Ok(commits) => {
            if commits.is_empty() {
                return ok("no commits yet".to_string());
            }
            let lines: Vec<String> = commits
                .iter()
                .map(|c| format!("{} {} ({}, {}) {}", c.short_sha, c.subject, c.author, c.date, c.sha))
                .collect();
            ok(lines.join("\n"))
        }
        Err(e) => err(format!("git log failed: {e}")),
    }
}

// ---------------------------------------------------------------------
// send_message (M11) — agent-to-agent structured messages within a mission.
// Only reachable at all when `ctx.mission_context` is `Some` (see
// `agent::schema::send_message_tool_definition`'s own docs for why the model
// never even sees this tool otherwise); dispatch still handles the `None`
// case defensively rather than assuming that invariant always holds.
// ---------------------------------------------------------------------

/// Resolves `to_task_title` (if given) to the most recent `agent_run_id` of
/// the matching task in `mission_tasks` — best-effort, exactly as the M11
/// spec describes: `None` if no title was given, or if a matching task
/// exists but hasn't been started yet (its `agent_run_id` is still unset).
/// Pure — no I/O — so this is unit-tested directly, independent of
/// `send_message`'s own DB wiring. Returns `Err` only when a title was given
/// but no task in the mission has that exact title (a likely typo the model
/// can react to), distinct from "found the task, it just hasn't run yet"
/// (`Ok(None)`, not an error — the message is still recorded).
fn resolve_recipient_agent_run_id(mission_tasks: &[Task], to_task_title: Option<&str>) -> Result<Option<String>, String> {
    let Some(title) = to_task_title else { return Ok(None) };
    match mission_tasks.iter().find(|t| t.title == title) {
        Some(task) => Ok(task.agent_run_id.clone()),
        None => Err(format!(
            "no task titled '{title}' was found in this mission — check the exact task title (case-sensitive) and try again, or omit to_task_title to broadcast"
        )),
    }
}

fn send_message(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let Some(mission) = &ctx.mission_context else {
        return err(
            "send_message is only available when this run is executing as part of a mission — this run is a \
             standalone agent run with no mission context."
                .to_string(),
        );
    };
    let subject = match require_str(input, "subject") {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let body = match require_str(input, "body") {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let to_task_title = input.get("to_task_title").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());

    let conn = match ctx.db_pool.get() {
        Ok(c) => c,
        Err(e) => return err(format!("failed to open a database connection: {e}")),
    };
    let mission_tasks = match tasks_repo::list_for_mission(&conn, &mission.mission_id) {
        Ok(tasks) => tasks,
        Err(e) => return err(format!("failed to look up this mission's tasks: {e}")),
    };
    let to_agent_run_id = match resolve_recipient_agent_run_id(&mission_tasks, to_task_title) {
        Ok(id) => id,
        Err(e) => return err(e),
    };

    match agent_messages_repo::insert(&conn, &ctx.agent_run_id, to_agent_run_id.as_deref(), &mission.mission_id, subject, body) {
        Ok(_message) => ok(match to_task_title {
            Some(title) => format!("message sent to task \"{title}\""),
            None => "message broadcast to the whole mission".to_string(),
        }),
        Err(e) => err(format!("failed to record message: {e}")),
    }
}

// ---------------------------------------------------------------------
// Browser tools (M16) — one lazily-launched session per agent run, via
// `ctx.browser_manager`. Every call is scoped to `ctx.agent_run_id`;
// `BrowserManager` itself owns the actual session lifecycle (including the
// "one session per run, closed when the run ends" invariant — see
// `agent::tool_loop::finish_run`).
// ---------------------------------------------------------------------

async fn browser_open(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let url = match require_str(input, "url") {
        Ok(u) => u,
        Err(e) => return err(e),
    };
    match ctx.browser_manager.navigate(&ctx.agent_run_id, url).await {
        Ok(()) => ok(format!("opened '{url}'")),
        Err(e) => err(e.to_string()),
    }
}

async fn browser_click(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let selector = match require_str(input, "selector") {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    match ctx.browser_manager.click(&ctx.agent_run_id, selector).await {
        Ok(()) => ok(format!("clicked '{selector}'")),
        Err(e) => err(e.to_string()),
    }
}

async fn browser_type_tool(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let selector = match require_str(input, "selector") {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let text = match require_str(input, "text") {
        Ok(t) => t,
        Err(e) => return err(e),
    };
    match ctx.browser_manager.type_text(&ctx.agent_run_id, selector, text).await {
        Ok(()) => ok(format!("typed into '{selector}'")),
        Err(e) => err(e.to_string()),
    }
}

/// Captures a real screenshot via `ctx.browser_manager`, saves it to a new
/// file under `ctx.screenshots_dir`, and records a `visual_snapshots` row —
/// `kind` is decided here (`has_baseline_for_label`): the first screenshot
/// for a given `(run, label)` pair is the baseline, every later one with the
/// same label is a comparison. Every failure message is specific about
/// which part already happened (captured/saved to disk vs. not yet
/// recorded) rather than a bare "failed", since a screenshot genuinely
/// saved to disk but not yet recorded in the DB is a real (if degraded)
/// outcome the model should understand, not treated the same as nothing
/// having happened at all.
async fn browser_screenshot_tool(ctx: &ToolContext<'_>, input: &Value) -> ToolRunOutcome {
    let label = input.get("label").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("default");

    let bytes = match ctx.browser_manager.screenshot(&ctx.agent_run_id).await {
        Ok(b) => b,
        Err(e) => return err(e.to_string()),
    };

    if let Err(e) = std::fs::create_dir_all(&ctx.screenshots_dir) {
        return err(format!("failed to create the screenshots directory: {e}"));
    }
    let file_name = format!("{}-{}.png", ctx.agent_run_id, uuid::Uuid::new_v4());
    let file_path = ctx.screenshots_dir.join(&file_name);
    if let Err(e) = std::fs::write(&file_path, &bytes) {
        return err(format!("failed to save the screenshot to disk: {e}"));
    }
    let image_path = file_path.to_string_lossy().into_owned();

    let conn = match ctx.db_pool.get() {
        Ok(c) => c,
        Err(e) => {
            return err(format!(
                "the screenshot was captured and saved to '{image_path}', but failed to open a database \
                 connection to record it: {e}"
            ))
        }
    };
    let task_id = ctx.mission_context.as_ref().map(|m| m.task_id.as_str());
    let is_baseline = match visual_snapshots_repo::has_baseline_for_label(&conn, &ctx.agent_run_id, label) {
        Ok(has_one_already) => !has_one_already,
        Err(e) => {
            return err(format!(
                "the screenshot was captured and saved to '{image_path}', but failed to check for an existing \
                 baseline: {e}"
            ))
        }
    };
    let kind = if is_baseline { VisualSnapshotKind::Baseline } else { VisualSnapshotKind::Comparison };

    match visual_snapshots_repo::insert(&conn, &ctx.agent_run_id, task_id, label, &image_path, kind) {
        Ok(_) => ok(format!(
            "captured a {} screenshot for label '{label}', saved to '{image_path}'",
            if is_baseline { "baseline" } else { "comparison" }
        )),
        Err(e) => err(format!("the screenshot was captured and saved to '{image_path}', but failed to record it: {e}")),
    }
}

// ---------------------------------------------------------------------
// report_completion — ends the run rather than producing a tool_result.
// ---------------------------------------------------------------------

fn report_completion(input: &Value) -> ToolRunOutcome {
    let summary = input.get("summary").and_then(Value::as_str).unwrap_or("(no summary provided)").to_string();
    let success = input.get("success").and_then(Value::as_bool).unwrap_or(false);
    ToolRunOutcome::Completion { summary, success }
}

/// Measures how long an async tool call took, in milliseconds — used by
/// `agent::tool_loop` when writing the `tool_calls.duration_ms` column.
pub fn elapsed_ms(start: Instant) -> i64 {
    start.elapsed().as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::FakeBrowserBackend;
    use crate::git::{BranchInfo, CommitInfo, GitFileDiff, GitStatus, WorktreeInfo};
    use crate::os_adapter::OperatingSystemAdapter;
    use std::collections::HashMap;

    struct FakeGitService;
    impl GitService for FakeGitService {
        fn status(&self, _repo_path: &Path) -> crate::error::AppResult<GitStatus> {
            Ok(GitStatus::default())
        }
        fn branches(&self, _repo_path: &Path) -> crate::error::AppResult<Vec<BranchInfo>> {
            Ok(vec![])
        }
        fn current_branch(&self, _repo_path: &Path) -> crate::error::AppResult<Option<String>> {
            Ok(None)
        }
        fn is_git_repository(&self, _path: &Path) -> bool {
            true
        }
        fn init(&self, _path: &Path) -> crate::error::AppResult<()> {
            Ok(())
        }
        fn add_worktree(&self, _r: &Path, _w: &Path, _b: &str, _base: &str) -> crate::error::AppResult<()> {
            Ok(())
        }
        fn remove_worktree(&self, _r: &Path, _w: &Path) -> crate::error::AppResult<()> {
            Ok(())
        }
        fn list_worktrees(&self, _r: &Path) -> crate::error::AppResult<Vec<WorktreeInfo>> {
            Ok(vec![])
        }
        fn diff_file(&self, _repo_root: &Path, _path: &str) -> crate::error::AppResult<GitFileDiff> {
            Ok(GitFileDiff { original: String::new(), modified: String::new(), is_new_file: true, is_deleted: false })
        }
        fn log(&self, _repo_root: &Path, _limit: u32) -> crate::error::AppResult<Vec<CommitInfo>> {
            Ok(vec![])
        }
        fn conflicted_files(&self, _repo_root: &Path) -> crate::error::AppResult<Vec<crate::git::ConflictedFile>> {
            Ok(vec![])
        }
        fn merge_branch(&self, _repo_root: &Path, _branch: &str, _into: &str) -> crate::error::AppResult<crate::git::MergeOutcome> {
            Ok(crate::git::MergeOutcome::Clean)
        }
        fn merge_conflict_dry_run(&self, _repo_root: &Path, _branch: &str, _into: &str) -> crate::error::AppResult<crate::git::MergeOutcome> {
            Ok(crate::git::MergeOutcome::Clean)
        }
        fn abort_merge(&self, _repo_root: &Path) -> crate::error::AppResult<()> {
            Ok(())
        }
        fn stage_paths(&self, _repo_root: &Path, _paths: &[String]) -> crate::error::AppResult<()> {
            Ok(())
        }
        fn commit(&self, _repo_root: &Path, _message: &str) -> crate::error::AppResult<()> {
            Ok(())
        }
    }

    struct FakeOsAdapter;
    impl OperatingSystemAdapter for FakeOsAdapter {
        fn default_shell(&self) -> String {
            if cfg!(windows) { "powershell.exe".to_string() } else { "/bin/sh".to_string() }
        }
        fn shell_invocation(&self) -> Vec<String> {
            vec![self.default_shell()]
        }
        fn one_shot_shell_invocation(&self, command: &str) -> Vec<String> {
            if cfg!(windows) {
                vec!["powershell.exe".to_string(), "-NoLogo".to_string(), "-NonInteractive".to_string(), "-Command".to_string(), command.to_string()]
            } else {
                vec!["/bin/sh".to_string(), "-c".to_string(), command.to_string()]
            }
        }
        fn resolve_executable(&self, _name: &str) -> Option<PathBuf> {
            None
        }
        fn env_vars(&self) -> HashMap<String, String> {
            HashMap::new()
        }
        fn home_dir(&self) -> Option<PathBuf> {
            None
        }
    }

    /// A fresh, migrated in-memory DB pool for `send_message` tests.
    /// `max_size(1)` (rather than `r2d2`'s default of several) is what makes
    /// this work at all: `SqliteConnectionManager::memory()` gives each new
    /// physical connection its own separate, empty in-memory database, so a
    /// pool that could hand out more than one distinct connection would look
    /// like data vanishing between calls. Capped at one, `.get()` always
    /// hands back the same single connection (returned to the pool when its
    /// guard drops), so state written through one `.get()` call is still
    /// there on the next.
    fn test_db_pool() -> DbPool {
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(1).build(manager).expect("build in-memory pool");
        {
            let mut conn = pool.get().expect("get conn");
            crate::db::migrations::run_migrations(&mut conn).expect("run migrations");
        }
        pool
    }

    fn test_ctx<'a>(workspace_root: PathBuf, git: &'a FakeGitService, os: &'a FakeOsAdapter) -> ToolContext<'a> {
        test_ctx_with_pool(test_db_pool(), workspace_root, git, os)
    }

    /// Same as `test_ctx`, but takes an existing pool so a test can seed
    /// rows (a `projects` row for `run_tests_tool`'s `test_runs` writes,
    /// say) into the exact same in-memory DB the returned `ToolContext`
    /// will read/write through.
    fn test_ctx_with_pool<'a>(pool: DbPool, workspace_root: PathBuf, git: &'a FakeGitService, os: &'a FakeOsAdapter) -> ToolContext<'a> {
        // A subdirectory of the test's own `workspace_root` tempdir, so its
        // lifetime is tied to whatever `tempfile::TempDir` the caller
        // already keeps alive for the duration of the test — not a real
        // production layout (which uses the app-data directory globally),
        // just a scratch location for these dispatch-logic tests.
        let screenshots_dir = workspace_root.join(".test-screenshots");
        ToolContext {
            workspace_root,
            git_service: git,
            os_adapter: os,
            test_command: None,
            lint_command: None,
            build_command: None,
            // 20s, not a tight 5s: several tests here spawn a *real* OS shell
            // (`powershell.exe` on Windows especially has real, sometimes
            // multi-second, cold-start latency of its own), and CI runners
            // run the whole test binary's tests concurrently — under CPU
            // contention from every other test running at once, 5s was
            // observed to be tight enough to occasionally time out a shell
            // spawn that would otherwise finish near-instantly, failing the
            // test with `TimedOut` rather than the real (successful) result.
            // Tests that specifically exercise the timeout path (see
            // `run_command_times_out_and_is_killed`) override this field
            // directly with a much smaller value, so this default being
            // generous doesn't weaken that coverage at all.
            tool_timeout: Duration::from_secs(20),
            cancel: CancellationToken::new(),
            agent_run_id: "test-run".to_string(),
            mission_context: None,
            db_pool: pool,
            project_id: "p1".to_string(),
            browser_manager: Arc::new(BrowserManager::new(Arc::new(FakeBrowserBackend::default()))),
            screenshots_dir,
        }
    }

    /// Same as `test_ctx_with_pool`, but wires `browser_manager` to a
    /// `BrowserManager` over the given `backend` (rather than a fresh,
    /// unobservable `FakeBrowserBackend`) so a test can inspect what the
    /// browser tools actually called on it afterward.
    fn test_ctx_with_browser<'a>(
        pool: DbPool,
        workspace_root: PathBuf,
        git: &'a FakeGitService,
        os: &'a FakeOsAdapter,
        backend: Arc<FakeBrowserBackend>,
    ) -> ToolContext<'a> {
        let mut ctx = test_ctx_with_pool(pool, workspace_root, git, os);
        ctx.browser_manager = Arc::new(BrowserManager::new(backend));
        ctx
    }

    #[tokio::test]
    async fn read_file_rejects_path_traversal_outside_workspace() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "read_file", &serde_json::json!({"path": "../../../etc/passwd"})).await;
        match outcome {
            ToolRunOutcome::Result { output, is_error, .. } => {
                assert!(is_error, "path traversal must come back as a tool error, not succeed");
                assert!(output.contains(".."));
            }
            ToolRunOutcome::Completion { .. } => panic!("read_file must never end the run"),
        }
    }

    #[tokio::test]
    async fn write_then_read_round_trips_inside_workspace() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let write = dispatch_tool(&ctx, "write_file", &serde_json::json!({"path": "notes/a.txt", "content": "hello"})).await;
        assert!(matches!(write, ToolRunOutcome::Result { is_error: false, .. }));

        let read = dispatch_tool(&ctx, "read_file", &serde_json::json!({"path": "notes/a.txt"})).await;
        match read {
            ToolRunOutcome::Result { output, is_error, .. } => {
                assert!(!is_error);
                assert_eq!(output, "hello");
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn edit_file_requires_unique_old_string() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("f.txt"), "foo bar foo").unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "edit_file", &serde_json::json!({"path": "f.txt", "old_string": "foo", "new_string": "baz"})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("2 times"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn edit_file_replaces_unique_match() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("f.txt"), "foo bar baz").unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "edit_file", &serde_json::json!({"path": "f.txt", "old_string": "bar", "new_string": "qux"})).await;
        assert!(matches!(outcome, ToolRunOutcome::Result { is_error: false, .. }));
        let contents = std::fs::read_to_string(ws.path().join("f.txt")).unwrap();
        assert_eq!(contents, "foo qux baz");
    }

    #[tokio::test]
    async fn run_tests_without_configured_command_is_a_clear_tool_error() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "run_tests", &serde_json::json!({})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("no test command configured"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    /// M13: `run_tests` must persist a real `test_runs` row (the same table
    /// `commands::testing_commands::run_test_suite`'s manual runs write to)
    /// and hand its id back as `test_run_id`, so `agent::tool_loop`'s
    /// `test_fix_cycle` activity events can reference the actual run rather
    /// than duplicating its output.
    #[tokio::test]
    async fn run_tests_persists_a_passing_test_runs_row_and_returns_its_id() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        {
            let conn = pool.get().unwrap();
            conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        }
        let mut ctx = test_ctx_with_pool(pool.clone(), ws.path().to_path_buf(), &git, &os);
        ctx.test_command = Some(if cfg!(windows) { "exit 0".to_string() } else { "true".to_string() });

        let outcome = dispatch_tool(&ctx, "run_tests", &serde_json::json!({})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, test_run_id, .. } => {
                assert!(!is_error, "run_tests unexpectedly failed: {output}");
                let id = test_run_id.expect("run_tests must persist a test_runs row and return its id");
                let conn = pool.get().unwrap();
                let run = crate::db::repository::test_runs::get_by_id(&conn, &id).unwrap().expect("test_runs row exists");
                assert_eq!(run.status, crate::db::models::TestRunStatus::Success);
                assert_eq!(run.project_id, "p1");
                assert_eq!(run.kind, crate::db::models::TestRunKind::Test);
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn run_tests_persists_a_failing_test_runs_row_too() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        {
            let conn = pool.get().unwrap();
            conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        }
        let mut ctx = test_ctx_with_pool(pool.clone(), ws.path().to_path_buf(), &git, &os);
        ctx.test_command = Some(if cfg!(windows) { "exit 1".to_string() } else { "false".to_string() });

        let outcome = dispatch_tool(&ctx, "run_tests", &serde_json::json!({})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, test_run_id, .. } => {
                assert!(is_error);
                let id = test_run_id.expect("even a failing run_tests call should still persist a row");
                let conn = pool.get().unwrap();
                let run = crate::db::repository::test_runs::get_by_id(&conn, &id).unwrap().expect("test_runs row exists");
                assert_eq!(run.status, crate::db::models::TestRunStatus::Failure);
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn report_completion_ends_the_run() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "report_completion", &serde_json::json!({"summary": "done", "success": true})).await;
        match outcome {
            ToolRunOutcome::Completion { summary, success } => {
                assert_eq!(summary, "done");
                assert!(success);
            }
            ToolRunOutcome::Result { .. } => panic!("report_completion must end the run"),
        }
    }

    #[test]
    fn glob_matches_extension_pattern_anywhere_in_tree() {
        let regex = regex::Regex::new(&glob_to_regex("*.rs")).unwrap();
        assert!(regex.is_match("main.rs"));
        assert!(!regex.is_match("main.rsx"));
    }

    #[test]
    fn glob_double_star_matches_nested_directories() {
        let regex = regex::Regex::new(&glob_to_regex("src/**/*.rs")).unwrap();
        assert!(regex.is_match("src/agent/tools.rs"));
        assert!(!regex.is_match("other/agent/tools.rs"));
    }

    #[tokio::test]
    async fn search_files_finds_nested_file_by_extension() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(ws.path().join("src/nested")).unwrap();
        std::fs::write(ws.path().join("src/nested/thing.rs"), "fn main() {}").unwrap();
        std::fs::write(ws.path().join("readme.md"), "hi").unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "search_files", &serde_json::json!({"pattern": "*.rs"})).await;
        match outcome {
            ToolRunOutcome::Result { output, is_error, .. } => {
                assert!(!is_error);
                assert!(output.contains("thing.rs"));
                assert!(!output.contains("readme.md"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn search_code_finds_substring_match_with_line_number() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("a.txt"), "line one\nTARGET line\nline three").unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "search_code", &serde_json::json!({"query": "TARGET"})).await;
        match outcome {
            ToolRunOutcome::Result { output, is_error, .. } => {
                assert!(!is_error);
                assert!(output.contains("a.txt:2:"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn run_command_times_out_and_is_killed() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let mut ctx = test_ctx(ws.path().to_path_buf(), &git, &os);
        ctx.tool_timeout = Duration::from_millis(200);

        let sleep_command = if cfg!(windows) { "Start-Sleep -Seconds 30" } else { "sleep 30" };
        let start = Instant::now();
        let outcome = dispatch_tool(&ctx, "run_command", &serde_json::json!({"command": sleep_command})).await;
        assert!(start.elapsed() < Duration::from_secs(10), "the timeout must actually cut the command short");
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("timed out"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn run_command_cancellation_kills_the_process_promptly() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);
        ctx.cancel.cancel();

        let sleep_command = if cfg!(windows) { "Start-Sleep -Seconds 30" } else { "sleep 30" };
        let start = Instant::now();
        let outcome = dispatch_tool(&ctx, "run_command", &serde_json::json!({"command": sleep_command})).await;
        assert!(start.elapsed() < Duration::from_secs(10), "cancellation must not wait for the process to finish");
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("cancelled"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    // -- send_message (M11) -------------------------------------------------

    /// Seeds the rows `send_message` needs: a project/repo/agent, a mission
    /// with two tasks ("Sender" already linked to `run1`, "Recipient" not
    /// yet started — `agent_run_id` still `NULL`), and the two `agent_runs`
    /// rows themselves.
    fn seed_mission_with_two_tasks(pool: &DbPool) {
        let conn = pool.get().expect("get conn");
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
        conn.execute("INSERT INTO missions (id, project_id, objective) VALUES ('m1', 'p1', 'Ship it')", []).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'sender task', 'claude-sonnet-5')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, mission_id, title, status, priority, position, agent_run_id) \
             VALUES ('t-sender', 'p1', 'm1', 'Sender', 'in_progress', 'medium', 0, 'run1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, mission_id, title, status, priority, position, agent_run_id) \
             VALUES ('t-recipient', 'p1', 'm1', 'Recipient', 'backlog', 'medium', 1, NULL)",
            [],
        )
        .unwrap();
    }

    fn mission_ctx<'a>(pool: DbPool, ws: PathBuf, git: &'a FakeGitService, os: &'a FakeOsAdapter) -> ToolContext<'a> {
        let screenshots_dir = ws.join(".test-screenshots");
        ToolContext {
            workspace_root: ws,
            git_service: git,
            os_adapter: os,
            test_command: None,
            lint_command: None,
            build_command: None,
            // See `test_ctx_with_pool`'s own comment on this same value.
            tool_timeout: Duration::from_secs(20),
            cancel: CancellationToken::new(),
            agent_run_id: "run1".to_string(),
            mission_context: Some(MissionContext { mission_id: "m1".to_string(), task_id: "t-sender".to_string() }),
            db_pool: pool,
            project_id: "p1".to_string(),
            browser_manager: Arc::new(BrowserManager::new(Arc::new(FakeBrowserBackend::default()))),
            screenshots_dir,
        }
    }

    #[tokio::test]
    async fn send_message_without_mission_context_is_a_clear_tool_error() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        // Reuses the ordinary (non-mission) test_ctx — mission_context: None.
        let ctx = test_ctx(ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "send_message", &serde_json::json!({"subject": "hi", "body": "body"})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("mission"), "error should explain why: {output}");
            }
            ToolRunOutcome::Completion { .. } => panic!("send_message must never end the run"),
        }
    }

    #[tokio::test]
    async fn send_message_broadcasts_when_to_task_title_is_omitted() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        seed_mission_with_two_tasks(&pool);
        let ctx = mission_ctx(pool.clone(), ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(&ctx, "send_message", &serde_json::json!({"subject": "Status", "body": "On track"})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(!is_error, "{output}");
                assert!(output.contains("broadcast"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }

        let conn = pool.get().unwrap();
        let messages = crate::db::repository::agent_messages::list_for_mission(&conn, "m1").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].from_agent_run_id, "run1");
        assert!(messages[0].to_agent_run_id.is_none());
        assert_eq!(messages[0].subject, "Status");
    }

    #[tokio::test]
    async fn send_message_resolves_to_task_title_to_that_tasks_agent_run_id() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        seed_mission_with_two_tasks(&pool);
        // Link the recipient task to a real run so resolution has something
        // to find.
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run2', 'a1', 'recipient task', 'claude-sonnet-5')",
                [],
            )
            .unwrap();
            conn.execute("UPDATE tasks SET agent_run_id = 'run2' WHERE id = 't-recipient'", []).unwrap();
        }
        let ctx = mission_ctx(pool.clone(), ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(
            &ctx,
            "send_message",
            &serde_json::json!({"subject": "Handoff", "body": "Schema is ready", "to_task_title": "Recipient"}),
        )
        .await;
        assert!(matches!(outcome, ToolRunOutcome::Result { is_error: false, .. }));

        let conn = pool.get().unwrap();
        let messages = crate::db::repository::agent_messages::list_for_mission(&conn, "m1").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].to_agent_run_id.as_deref(), Some("run2"));
    }

    #[tokio::test]
    async fn send_message_to_a_task_that_has_not_run_yet_still_records_the_message() {
        // Spec: "if that task hasn't run yet or already finished, still
        // record the message" — best-effort, not a live-chat requirement.
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        seed_mission_with_two_tasks(&pool); // "Recipient" has no agent_run_id yet.
        let ctx = mission_ctx(pool.clone(), ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(
            &ctx,
            "send_message",
            &serde_json::json!({"subject": "Heads up", "body": "starting soon", "to_task_title": "Recipient"}),
        )
        .await;
        match outcome {
            ToolRunOutcome::Result { is_error, .. } => assert!(!is_error, "recording against a not-yet-started task must still succeed"),
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }

        let conn = pool.get().unwrap();
        let messages = crate::db::repository::agent_messages::list_for_mission(&conn, "m1").unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].to_agent_run_id.is_none(), "the task exists but has no run yet, so there's nothing to link to");
    }

    #[tokio::test]
    async fn send_message_with_an_unknown_task_title_is_a_clear_tool_error() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        seed_mission_with_two_tasks(&pool);
        let ctx = mission_ctx(pool.clone(), ws.path().to_path_buf(), &git, &os);

        let outcome = dispatch_tool(
            &ctx,
            "send_message",
            &serde_json::json!({"subject": "hi", "body": "body", "to_task_title": "Nonexistent Task"}),
        )
        .await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("Nonexistent Task"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }

        let conn = pool.get().unwrap();
        let messages = crate::db::repository::agent_messages::list_for_mission(&conn, "m1").unwrap();
        assert!(messages.is_empty(), "a bad title should not record a phantom message");
    }

    // -- resolve_recipient_agent_run_id (pure) -------------------------------

    fn task_with_run(id: &str, title: &str, agent_run_id: Option<&str>) -> Task {
        use crate::db::models::{TaskPriority, TaskStatus};
        Task {
            id: id.to_string(),
            project_id: "p1".to_string(),
            mission_id: Some("m1".to_string()),
            title: title.to_string(),
            description: None,
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            position: 0,
            depends_on_task_id: None,
            agent_type: None,
            agent_run_id: agent_run_id.map(str::to_string),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn resolve_recipient_returns_none_without_a_title() {
        let tasks = vec![task_with_run("t1", "A", Some("run1"))];
        assert_eq!(resolve_recipient_agent_run_id(&tasks, None), Ok(None));
    }

    #[test]
    fn resolve_recipient_finds_the_matching_tasks_agent_run_id() {
        let tasks = vec![task_with_run("t1", "A", Some("run1")), task_with_run("t2", "B", Some("run2"))];
        assert_eq!(resolve_recipient_agent_run_id(&tasks, Some("B")), Ok(Some("run2".to_string())));
    }

    #[test]
    fn resolve_recipient_is_ok_none_for_a_task_that_has_not_run_yet() {
        let tasks = vec![task_with_run("t1", "A", None)];
        assert_eq!(resolve_recipient_agent_run_id(&tasks, Some("A")), Ok(None));
    }

    #[test]
    fn resolve_recipient_errors_for_an_unknown_title() {
        let tasks = vec![task_with_run("t1", "A", Some("run1"))];
        assert!(resolve_recipient_agent_run_id(&tasks, Some("does not exist")).is_err());
    }

    // -- browser tools (M16) -------------------------------------------------

    #[tokio::test]
    async fn browser_open_dispatches_navigate_through_the_browser_manager() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let backend = Arc::new(FakeBrowserBackend::default());
        let ctx = test_ctx_with_browser(test_db_pool(), ws.path().to_path_buf(), &git, &os, backend.clone());

        let outcome = dispatch_tool(&ctx, "browser_open", &serde_json::json!({"url": "https://example.com"})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(!is_error, "{output}");
                assert!(output.contains("example.com"));
            }
            ToolRunOutcome::Completion { .. } => panic!("browser_open must never end the run"),
        }

        let navigate_calls = backend.navigate_calls.lock().unwrap();
        assert_eq!(navigate_calls.len(), 1);
        assert_eq!(navigate_calls[0].1, "https://example.com");
    }

    #[tokio::test]
    async fn browser_click_and_browser_type_dispatch_through_the_browser_manager_and_share_one_session() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let backend = Arc::new(FakeBrowserBackend::default());
        let ctx = test_ctx_with_browser(test_db_pool(), ws.path().to_path_buf(), &git, &os, backend.clone());

        let click = dispatch_tool(&ctx, "browser_click", &serde_json::json!({"selector": "button.submit"})).await;
        assert!(matches!(click, ToolRunOutcome::Result { is_error: false, .. }));
        let typed = dispatch_tool(&ctx, "browser_type", &serde_json::json!({"selector": "input#email", "text": "a@b.com"})).await;
        assert!(matches!(typed, ToolRunOutcome::Result { is_error: false, .. }));

        let click_calls = backend.click_calls.lock().unwrap();
        let type_calls = backend.type_calls.lock().unwrap();
        assert_eq!(click_calls[0].1, "button.submit");
        assert_eq!((type_calls[0].1.as_str(), type_calls[0].2.as_str()), ("input#email", "a@b.com"));
        assert_eq!(click_calls[0].0, type_calls[0].0, "both calls in the same run must reuse the same browser session");
    }

    #[tokio::test]
    async fn browser_open_surfaces_a_launch_failure_as_a_clear_tool_error() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let backend = Arc::new(FakeBrowserBackend { fail_launch: true, ..Default::default() });
        let ctx = test_ctx_with_browser(test_db_pool(), ws.path().to_path_buf(), &git, &os, backend);

        let outcome = dispatch_tool(&ctx, "browser_open", &serde_json::json!({"url": "https://example.com"})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("Chrome"), "must be the honest 'no browser found' message: {output}");
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn browser_screenshot_saves_a_real_file_under_screenshots_dir_and_records_a_baseline_row() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        {
            let conn = pool.get().unwrap();
            conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
            conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
            conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
            conn.execute(
                "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('test-run', 'a1', 'do it', 'claude-sonnet-5')",
                [],
            )
            .unwrap();
        }
        let backend = Arc::new(FakeBrowserBackend { screenshot_bytes: vec![1, 2, 3, 4], ..Default::default() });
        let ctx = test_ctx_with_browser(pool.clone(), ws.path().to_path_buf(), &git, &os, backend);

        let outcome = dispatch_tool(&ctx, "browser_screenshot", &serde_json::json!({"label": "home"})).await;
        match outcome {
            ToolRunOutcome::Result { is_error, output, .. } => assert!(!is_error, "{output}"),
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }

        let conn = pool.get().unwrap();
        let rows = crate::db::repository::visual_snapshots::list_for_run(&conn, "test-run").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "home");
        assert_eq!(rows[0].kind, crate::db::models::VisualSnapshotKind::Baseline);
        assert!(
            std::path::Path::new(&rows[0].image_path).starts_with(&ctx.screenshots_dir),
            "the saved file must live under ctx.screenshots_dir: {}",
            rows[0].image_path
        );
        let saved_bytes = std::fs::read(&rows[0].image_path).unwrap();
        assert_eq!(saved_bytes, vec![1, 2, 3, 4], "the file on disk must be exactly what the browser backend captured");
    }

    #[tokio::test]
    async fn browser_screenshot_second_call_with_same_label_is_a_comparison_not_another_baseline() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        {
            let conn = pool.get().unwrap();
            conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
            conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
            conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
            conn.execute(
                "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('test-run', 'a1', 'do it', 'claude-sonnet-5')",
                [],
            )
            .unwrap();
        }
        let backend = Arc::new(FakeBrowserBackend::default());
        let ctx = test_ctx_with_browser(pool.clone(), ws.path().to_path_buf(), &git, &os, backend);

        dispatch_tool(&ctx, "browser_screenshot", &serde_json::json!({"label": "home"})).await;
        dispatch_tool(&ctx, "browser_screenshot", &serde_json::json!({"label": "home"})).await;

        let conn = pool.get().unwrap();
        let rows = crate::db::repository::visual_snapshots::list_for_run(&conn, "test-run").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, crate::db::models::VisualSnapshotKind::Baseline);
        assert_eq!(rows[1].kind, crate::db::models::VisualSnapshotKind::Comparison);
    }

    #[tokio::test]
    async fn browser_screenshot_defaults_label_to_default_when_omitted() {
        let ws = tempfile::tempdir().unwrap();
        let git = FakeGitService;
        let os = FakeOsAdapter;
        let pool = test_db_pool();
        {
            let conn = pool.get().unwrap();
            conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
            conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
            conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
            conn.execute(
                "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('test-run', 'a1', 'do it', 'claude-sonnet-5')",
                [],
            )
            .unwrap();
        }
        let backend = Arc::new(FakeBrowserBackend::default());
        let ctx = test_ctx_with_browser(pool.clone(), ws.path().to_path_buf(), &git, &os, backend);

        dispatch_tool(&ctx, "browser_screenshot", &serde_json::json!({})).await;

        let conn = pool.get().unwrap();
        let rows = crate::db::repository::visual_snapshots::list_for_run(&conn, "test-run").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "default");
    }
}

//! M15: a bounded, narrowly-scoped AI conflict resolver — offered as an
//! explicit fallback action (never automatic) when `commands::merge_commands
//! ::merge_agent_run` leaves the primary checkout in a conflicted merge
//! state. Reuses the exact same tool-calling machinery M6's main loop and
//! M14's reviewer already use (`AnthropicClient`, `agent::tools::
//! dispatch_tool`, `agent::tool_loop::to_content_block_param`) — not a new
//! engine — restricted to `agent::schema::conflict_resolver_tool_definitions`
//! (`read_file`/`edit_file`/`git_status`/`git_diff` only) and, beyond that
//! tool list, further scoped so `read_file`/`edit_file` may only touch the
//! exact set of files git itself reported as conflicted when this started —
//! enforced here by [`resolve_conflicts_with_agent`] itself (see
//! [`scoped_dispatch`]), not merely asked for in the system prompt.
//!
//! ## Iteration cap
//!
//! [`MAX_ITERATIONS`] (12) is deliberately its own, much smaller budget than
//! `agent::tool_loop::DEFAULT_MAX_ITERATIONS` (40): this agent isn't doing
//! open-ended work, it's resolving conflict markers in a small, already-known
//! set of files — reading each one, editing it, and a couple of follow-up
//! `git_status`/`git_diff` checks is realistically enough turns for that,
//! and a much larger budget would just let a stuck model spin.
//!
//! ## Never committing a bad resolution
//!
//! The loop ends when the model stops calling tools on its own (the same
//! "no forced end tool" pattern `agent::reviewer`'s context-gathering phase
//! uses) or when [`MAX_ITERATIONS`] is exhausted — either way, what happens
//! next never trusts the model's own say-so, and runs as two ordered gates
//! (order matters: git's own unmerged/`U` index state for a path doesn't
//! clear just because its on-disk markers are gone — only `git add`-ing it
//! does that):
//!
//! 1. A literal-text check on every originally-conflicted file's on-disk
//!    content for `<<<<<<<`/`=======`/`>>>>>>>` markers — the only
//!    meaningful check available before anything is staged. Any marker
//!    anywhere leaves the merge untouched and returns a clear `Err`.
//! 2. Only once every file passes (1) are the files staged
//!    (`GitService::stage_paths`) — and then, immediately before the commit
//!    that finishes the merge, a **second**, independent,
//!    `GitService::conflicted_files` (real `git status --porcelain=v2`)
//!    check confirms git itself now agrees nothing is unmerged. Only then
//!    does `GitService::commit` run.
//!
//! Any failure at either gate leaves the merge exactly as conflicted as it
//! already was — never a bad commit.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::db::repository::{agent_runs as agent_runs_repo, agents as agents_repo, model_configs as model_configs_repo, repositories as repositories_repo, workspaces as workspaces_repo};
use crate::db::DbConnection;
use crate::error::{AppError, AppResult};
use crate::git::{ConflictedFile, GitCliService, GitService};
use crate::os_adapter;
use crate::secrets;
use crate::state::AppState;

use super::anthropic_client::{AnthropicClient, ContentBlockParam, MessageParam, StreamOutcome};
use super::schema::conflict_resolver_tool_definitions;
use super::tool_loop::to_content_block_param;
use super::tools::{dispatch_tool, MissionContext as ToolMissionContext, ToolContext, ToolRunOutcome};

/// See this module's own docs for why this is so much smaller than
/// `agent::tool_loop::DEFAULT_MAX_ITERATIONS`.
const MAX_ITERATIONS: i64 = 12;
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_TOKENS: u32 = 8192;

fn get_conn(app: &AppHandle) -> AppResult<DbConnection> {
    app.state::<AppState>().db.get().map_err(Into::into)
}

/// True if `text` contains any of git's own conflict-marker lines — the
/// independent, literal-text check [`resolve_conflicts_with_agent`] runs on
/// every originally-conflicted file in addition to trusting
/// `GitService::conflicted_files`'s real `git status` parsing, before ever
/// staging/committing anything.
fn contains_conflict_markers(text: &str) -> bool {
    text.lines().any(|line| line.starts_with("<<<<<<<") || line.starts_with("=======") || line.starts_with(">>>>>>>"))
}

/// Normalizes a tool-provided path the same way `agent::tools::git_diff`
/// does before comparing it against the conflicted-file allowlist — so a
/// Windows-style `\`-separated path (or a leading `./`) doesn't sneak past
/// the scope check on a technicality.
fn normalize_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized.strip_prefix("./").map(str::to_string).unwrap_or(normalized)
}

/// Runs `read_file`/`edit_file` through the ordinary `dispatch_tool`, but
/// only if their `path` input is one of `allowed_paths` — every other tool
/// (`git_status`/`git_diff`) passes straight through. This is the actual
/// enforcement of "scoped to only the conflicted files": a model that tries
/// to read or edit anything else gets a clear tool error, not a silent
/// no-op or a real file access.
async fn scoped_dispatch(ctx: &ToolContext<'_>, name: &str, input: &serde_json::Value, allowed_paths: &HashSet<String>) -> ToolRunOutcome {
    if matches!(name, "read_file" | "edit_file") {
        let path = input.get("path").and_then(serde_json::Value::as_str).unwrap_or("");
        if !allowed_paths.contains(&normalize_path(path)) {
            return ToolRunOutcome::Result {
                output: format!(
                    "'{path}' is not one of the conflicted files this resolver is scoped to ({}) — only those files may be read or edited",
                    allowed_paths.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
                is_error: true,
                test_run_id: None,
            };
        }
    }
    dispatch_tool(ctx, name, input).await
}

fn system_prompt(task_prompt: &str, primary_root: &str, base_branch: &str, agent_branch: &str, conflicts: &[ConflictedFile]) -> String {
    let file_list = conflicts.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join(", ");
    format!(
        "You are resolving real git merge conflicts in the primary checkout at `{primary_root}`, mid-merge of \
         branch `{agent_branch}` into `{base_branch}`. The original task this branch completed:\n{task_prompt}\n\n\
         Exactly these files are conflicted and need resolving — you may only read or edit these files, nothing \
         else: {file_list}\n\n\
         For each conflicted file: `read_file` it, find the `<<<<<<<`/`=======`/`>>>>>>>` conflict marker blocks, \
         and `edit_file` it to the correct resolved content with every marker removed (keep whichever side's \
         change is correct, or a sensible combination of both — never leave a marker in place). Use `git_diff`/\
         `git_status` if you need more context on what each side changed. When every listed file has no conflict \
         markers left, simply stop calling tools — do not call any other tool to \"finish\"; resolution is \
         verified independently after you stop.",
    )
}

/// Runs the bounded resolver loop end to end for `agent_run_id`, whose
/// primary checkout must currently be mid-conflicted-merge (verified for
/// real, not assumed) — reads/edits only the files that were conflicted when
/// this started, then independently verifies the resolution before staging
/// and committing to finish the merge. See this module's own docs for the
/// full safety story.
pub async fn resolve_conflicts_with_agent(app: &AppHandle, agent_run_id: &str) -> AppResult<()> {
    let (agent_run, agent, primary_root, base_branch, agent_branch) = {
        let conn = get_conn(app)?;
        let agent_run = agent_runs_repo::get_by_id(&conn, agent_run_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))?;
        let agent = agents_repo::get_by_id(&conn, &agent_run.agent_id)?
            .ok_or_else(|| AppError::NotFound(format!("agent {} not found", agent_run.agent_id)))?;
        let workspace_id = agent_run
            .workspace_id
            .clone()
            .ok_or_else(|| AppError::InvalidInput(format!("agent run {agent_run_id} has no workspace")))?;
        let workspace = workspaces_repo::get_by_id(&conn, &workspace_id)?
            .ok_or_else(|| AppError::NotFound(format!("workspace {workspace_id} not found")))?;
        let repository = repositories_repo::get_by_id(&conn, &workspace.repository_id)?
            .ok_or_else(|| AppError::NotFound(format!("repository {} not found", workspace.repository_id)))?;
        let base_branch = workspace
            .base_branch
            .clone()
            .ok_or_else(|| AppError::InvalidInput("this run's workspace has no recorded base branch".to_string()))?;
        (agent_run, agent, repository.root_path, base_branch, workspace.branch_name.clone())
    };

    let os_adapter = os_adapter::current();
    let git_service: Box<dyn GitService> = Box::new(GitCliService::new(os_adapter.as_ref()));
    let primary_root_path = PathBuf::from(&primary_root);

    // Never trust that a conflicted merge is actually in progress — check
    // for real before doing anything else (including spending an API call).
    let initial_conflicts = git_service.conflicted_files(&primary_root_path)?;
    if initial_conflicts.is_empty() {
        return Err(AppError::InvalidInput(
            "no conflicted merge is currently in progress in the primary checkout — nothing to resolve".to_string(),
        ));
    }
    let allowed_paths: HashSet<String> = initial_conflicts.iter().map(|f| normalize_path(&f.path)).collect();

    let api_key = tauri::async_runtime::spawn_blocking(|| secrets::get_secret(secrets::ANTHROPIC_API_KEY))
        .await
        .map_err(|e| AppError::Other(format!("API key lookup panicked: {e}")))??;
    let Some(api_key) = api_key else {
        return Err(AppError::InvalidInput(
            "No Anthropic API key is configured. Add one in Settings, then ask the agent to resolve conflicts again.".to_string(),
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

    let cancel = CancellationToken::new();
    let db_pool = app.state::<AppState>().db.clone();
    let ctx = ToolContext {
        workspace_root: primary_root_path.clone(),
        git_service: git_service.as_ref(),
        os_adapter: os_adapter.as_ref(),
        test_command: None,
        lint_command: None,
        build_command: None,
        tool_timeout: Duration::from_millis(DEFAULT_TOOL_TIMEOUT_MS),
        cancel: cancel.clone(),
        agent_run_id: agent_run_id.to_string(),
        // Never `send_message` here regardless — this tool isn't even
        // offered (see `conflict_resolver_tool_definitions`) — but kept
        // `None` for the same reason `agent::reviewer` does.
        mission_context: None::<ToolMissionContext>,
        db_pool,
        project_id: agent.project_id.clone(),
    };

    let system = system_prompt(&agent_run.task_prompt, &primary_root, &base_branch, &agent_branch, &initial_conflicts);
    let tools = conflict_resolver_tool_definitions();
    let mut messages = vec![MessageParam::user_text(
        "Resolve the conflicts now using the tools available to you, then stop calling tools once every listed file's \
         markers are gone.",
    )];

    for _ in 0..MAX_ITERATIONS {
        if cancel.is_cancelled() {
            return Err(AppError::Other("conflict resolution was cancelled".to_string()));
        }

        let outcome = client.stream_turn(&agent_run.model_id, max_tokens, &system, &messages, &tools, &cancel, |_text: &str| {}).await?;
        let turn = match outcome {
            StreamOutcome::Turn(turn) => turn,
            StreamOutcome::Cancelled => return Err(AppError::Other("conflict resolution was cancelled".to_string())),
        };

        let tool_uses: Vec<(String, String, serde_json::Value)> =
            turn.tool_uses().map(|(id, name, input)| (id.to_string(), name.to_string(), input.clone())).collect();
        let assistant_blocks: Vec<ContentBlockParam> = turn.content.iter().map(to_content_block_param).collect();
        messages.push(MessageParam::assistant(assistant_blocks));

        if tool_uses.is_empty() {
            // The model decided it's done — verified for real below, not
            // trusted here.
            break;
        }

        let mut tool_results: Vec<ContentBlockParam> = Vec::with_capacity(tool_uses.len());
        for (tool_use_id, tool_name, input) in tool_uses {
            if cancel.is_cancelled() {
                return Err(AppError::Other("conflict resolution was cancelled".to_string()));
            }
            let outcome = scoped_dispatch(&ctx, &tool_name, &input, &allowed_paths).await;
            let (output, is_error) = match outcome {
                ToolRunOutcome::Result { output, is_error, .. } => (output, is_error),
                // `report_completion` is never offered here (see
                // `conflict_resolver_tool_definitions`) — unreachable in
                // practice, handled defensively rather than panicking.
                ToolRunOutcome::Completion { summary, .. } => (summary, false),
            };
            tool_results.push(ContentBlockParam::ToolResult { tool_use_id, content: output, is_error });
        }
        messages.push(MessageParam::user_tool_results(tool_results));
    }

    finish_resolution(git_service.as_ref(), &primary_root_path, &initial_conflicts, &agent_branch, &base_branch)
}

/// The safety-critical tail of [`resolve_conflicts_with_agent`], split out
/// specifically so it's directly unit-testable against a real conflicted
/// tempdir repo without needing an Anthropic API call — this is where "never
/// commit a bad resolution" is actually enforced, so it's the part worth
/// testing in isolation. Never trusts the model's own say-so; runs two
/// ordered gates (order matters: git's own unmerged/`U` index state for a
/// path doesn't clear just because its on-disk markers are gone — only
/// `git add`-ing it does that):
///
/// 1. A literal-text check on every originally-conflicted file's on-disk
///    content for `<<<<<<<`/`=======`/`>>>>>>>` markers — the only
///    meaningful check available before anything is staged. Any marker
///    anywhere leaves the merge untouched and returns a clear `Err`.
/// 2. Only once every file passes (1) are the files staged
///    (`GitService::stage_paths`) — and then, immediately before the commit
///    that finishes the merge, a **second**, independent,
///    `GitService::conflicted_files` (real `git status --porcelain=v2`)
///    check confirms git itself now agrees nothing is unmerged. Only then
///    does `GitService::commit` run.
///
/// Any failure at either gate leaves the merge exactly as conflicted as it
/// already was — never a bad commit.
fn finish_resolution(
    git_service: &dyn GitService,
    primary_root: &std::path::Path,
    initial_conflicts: &[ConflictedFile],
    agent_branch: &str,
    base_branch: &str,
) -> AppResult<()> {
    // 1) Literal-text check, before anything is staged.
    for file in initial_conflicts {
        let full_path = primary_root.join(&file.path);
        let text = std::fs::read_to_string(&full_path)
            .map_err(|e| AppError::Other(format!("failed to read '{}' to verify its resolution: {e}", file.path)))?;
        if contains_conflict_markers(&text) {
            return Err(AppError::Other(format!(
                "the conflict resolver stopped without resolving every conflict — '{}' still contains conflict \
                 marker text. The merge is left as-is; resolve manually or abort the merge.",
                file.path
            )));
        }
    }

    // Markers are gone from every file — now (and only now) stage them, so
    // git's own status can actually reflect the resolution.
    let conflicted_paths: Vec<String> = initial_conflicts.iter().map(|f| f.path.clone()).collect();
    git_service.stage_paths(primary_root, &conflicted_paths)?;

    // 2) The real, independent, git-backed check, run fresh (not the
    //    `initial_conflicts` snapshot) immediately before the commit that
    //    finishes the merge — belt and suspenders beyond (1): if staging
    //    somehow left anything genuinely unmerged, this refuses to commit
    //    rather than trusting (1) alone.
    let remaining = git_service.conflicted_files(primary_root)?;
    if !remaining.is_empty() {
        let paths = remaining.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join(", ");
        return Err(AppError::Other(format!(
            "'{paths}' still show as unmerged after staging — refusing to commit an unresolved merge. \
             The merge is left as-is; resolve manually or abort the merge."
        )));
    }

    // Both checks passed — genuinely safe to finish the merge.
    git_service.commit(
        primary_root,
        &format!("Merge '{agent_branch}' into '{base_branch}' (conflicts resolved by AI conflict resolver)"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::MergeOutcome;

    // -- finish_resolution: real git, in a tempdir repo — this is the
    // safety-critical "never commit a bad resolution" logic, so it's tested
    // directly against real conflict state rather than only indirectly
    // through the (API-key-requiring) `resolve_conflicts_with_agent`.

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git").args(args).current_dir(dir).status().expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    /// A real tempdir repo, mid-conflicted-merge of a `feature` branch into
    /// its base branch over the same line of `shared.txt` — a genuine
    /// conflict from `GitCliService::merge_branch` itself, not simulated.
    fn setup_conflicted_repo() -> (tempfile::TempDir, GitCliService, String, String, Vec<ConflictedFile>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let os_adapter = os_adapter::current();
        let git_service = GitCliService::new(os_adapter.as_ref());

        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        std::fs::write(dir.path().join("shared.txt"), "base content\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init"]);
        let base_branch = git_service.current_branch(dir.path()).expect("current_branch").expect("not detached");

        git(dir.path(), &["checkout", "-b", "feature"]);
        std::fs::write(dir.path().join("shared.txt"), "feature branch's version\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "feature change"]);
        git(dir.path(), &["checkout", &base_branch]);

        std::fs::write(dir.path().join("shared.txt"), "base branch's version\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "base change"]);

        let outcome = git_service.merge_branch(dir.path(), "feature", &base_branch).expect("merge_branch");
        let conflicts = match outcome {
            MergeOutcome::Conflicts(files) => files,
            MergeOutcome::Clean => panic!("test setup should produce a real conflict"),
        };

        (dir, git_service, base_branch, "feature".to_string(), conflicts)
    }

    #[test]
    fn finish_resolution_refuses_to_commit_when_markers_remain() {
        let (repo_dir, git_service, base_branch, agent_branch, conflicts) = setup_conflicted_repo();

        // Don't touch the conflicted file at all — its markers are still
        // there, exactly as git left them.
        let result = finish_resolution(&git_service, repo_dir.path(), &conflicts, &agent_branch, &base_branch);

        let err = result.expect_err("must refuse to commit while markers remain");
        assert!(err.to_string().contains("marker"), "error should explain why: {err}");

        // Nothing was staged or committed — the merge is exactly as
        // conflicted as it was before this call.
        assert_eq!(git_service.conflicted_files(repo_dir.path()).expect("conflicted_files").len(), 1);
        let on_disk = std::fs::read_to_string(repo_dir.path().join("shared.txt")).expect("read shared.txt");
        assert!(on_disk.contains("<<<<<<<"), "on-disk markers must be untouched");
    }

    #[test]
    fn finish_resolution_also_refuses_when_only_some_markers_are_removed() {
        let (repo_dir, git_service, base_branch, agent_branch, conflicts) = setup_conflicted_repo();

        // Simulate a half-finished edit: the `<<<<<<<`/`=======` markers were
        // removed but a stray `>>>>>>>` was left behind — still a real,
        // unresolved conflict.
        std::fs::write(repo_dir.path().join("shared.txt"), "base branch's version\n>>>>>>> feature\n").unwrap();

        let result = finish_resolution(&git_service, repo_dir.path(), &conflicts, &agent_branch, &base_branch);
        assert!(result.is_err());
        assert_eq!(git_service.conflicted_files(repo_dir.path()).expect("conflicted_files").len(), 1, "must still be unmerged — nothing staged");
    }

    #[test]
    fn finish_resolution_stages_and_commits_once_markers_are_actually_gone() {
        let (repo_dir, git_service, base_branch, agent_branch, conflicts) = setup_conflicted_repo();

        // A genuine resolution: overwrite the file with real, marker-free
        // content (exactly what the AI resolver's `edit_file` calls would
        // produce).
        std::fs::write(repo_dir.path().join("shared.txt"), "the resolved content\n").expect("write resolved content");

        let result = finish_resolution(&git_service, repo_dir.path(), &conflicts, &agent_branch, &base_branch);
        assert!(result.is_ok(), "should succeed once every marker is genuinely gone: {result:?}");

        assert!(git_service.conflicted_files(repo_dir.path()).expect("conflicted_files").is_empty());
        let on_disk = std::fs::read_to_string(repo_dir.path().join("shared.txt")).expect("read shared.txt");
        assert_eq!(on_disk, "the resolved content\n");
        let log = git_service.log(repo_dir.path(), 5).expect("log");
        assert!(log.iter().any(|c| c.subject.contains("AI conflict resolver")), "the finishing commit should be a real commit");
    }

    #[test]
    fn contains_conflict_markers_detects_every_marker_line() {
        assert!(contains_conflict_markers("a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> feature\n"));
        // Each marker alone, on its own line, is also detected — a fully
        // resolved file shouldn't have any of these three lines left,
        // regardless of which one.
        assert!(contains_conflict_markers("=======\n"));
        assert!(contains_conflict_markers("<<<<<<< HEAD\n"));
        assert!(contains_conflict_markers(">>>>>>> feature\n"));
        // A line that merely *contains* "=======" somewhere other than at
        // its start (e.g. quoted in a comment, or indented) is intentionally
        // not flagged — git's own markers are always at the start of a line.
        assert!(!contains_conflict_markers("  // this line mentions ======= but isn't a real marker\n"));
        assert!(!contains_conflict_markers("no markers here at all\njust normal text\n"));
    }

    #[test]
    fn normalize_path_handles_backslashes_and_leading_dot_slash() {
        assert_eq!(normalize_path("src\\lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("./src/lib.rs"), "src/lib.rs");
        assert_eq!(normalize_path("src/lib.rs"), "src/lib.rs");
    }

    #[tokio::test]
    async fn scoped_dispatch_refuses_read_file_outside_the_allowed_set() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("allowed.txt"), "ok").unwrap();
        std::fs::write(ws.path().join("secret.txt"), "nope").unwrap();

        let os_adapter = os_adapter::current();
        let git_service = crate::git::GitCliService::new(os_adapter.as_ref());
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(1).build(manager).expect("build pool");
        {
            let mut conn = pool.get().unwrap();
            crate::db::migrations::run_migrations(&mut conn).unwrap();
        }
        let ctx = ToolContext {
            workspace_root: ws.path().to_path_buf(),
            git_service: &git_service,
            os_adapter: os_adapter.as_ref(),
            test_command: None,
            lint_command: None,
            build_command: None,
            tool_timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            agent_run_id: "run1".to_string(),
            mission_context: None,
            db_pool: pool,
            project_id: "p1".to_string(),
        };
        let allowed: HashSet<String> = ["allowed.txt".to_string()].into_iter().collect();

        let refused = scoped_dispatch(&ctx, "read_file", &serde_json::json!({"path": "secret.txt"}), &allowed).await;
        match refused {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(is_error);
                assert!(output.contains("not one of the conflicted files"));
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }

        let allowed_read = scoped_dispatch(&ctx, "read_file", &serde_json::json!({"path": "allowed.txt"}), &allowed).await;
        match allowed_read {
            ToolRunOutcome::Result { is_error, output, .. } => {
                assert!(!is_error);
                assert_eq!(output, "ok");
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }

    #[tokio::test]
    async fn scoped_dispatch_passes_git_status_through_unscoped() {
        let ws = tempfile::tempdir().unwrap();
        let os_adapter = os_adapter::current();
        let git_service = crate::git::GitCliService::new(os_adapter.as_ref());
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(1).build(manager).expect("build pool");
        {
            let mut conn = pool.get().unwrap();
            crate::db::migrations::run_migrations(&mut conn).unwrap();
        }
        let ctx = ToolContext {
            workspace_root: ws.path().to_path_buf(),
            git_service: &git_service,
            os_adapter: os_adapter.as_ref(),
            test_command: None,
            lint_command: None,
            build_command: None,
            tool_timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            agent_run_id: "run1".to_string(),
            mission_context: None,
            db_pool: pool,
            project_id: "p1".to_string(),
        };
        let allowed: HashSet<String> = HashSet::new();

        // git_status isn't file-scoped at all — it must never be refused by
        // the allowlist check (even though `allowed` is empty here), whether
        // or not `ws` happens to be a real git repo.
        let outcome = scoped_dispatch(&ctx, "git_status", &serde_json::json!({}), &allowed).await;
        match outcome {
            ToolRunOutcome::Result { output, .. } => {
                assert!(!output.contains("not one of the conflicted files"), "git_status must never be scope-refused");
            }
            ToolRunOutcome::Completion { .. } => panic!("unexpected completion"),
        }
    }
}

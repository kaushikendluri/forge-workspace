//! M15 commands: merge readiness, the real merge, aborting a conflicted
//! merge, and kicking off the bounded AI conflict resolver
//! (`agent::conflict_resolver`) for an agent run's completed work. Every
//! merge operation here lands in the repository's **primary** checkout
//! (`repositories.root_path` — the real checkout the user has open), never
//! in the agent's own disposable worktree; see `git::GitService::
//! merge_branch`'s own docs for why.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::agent::conflict_resolver;
use crate::commands::run_blocking;
use crate::db::models::{ReviewStatus, TestRunKind, TestRunStatus};
use crate::db::repository::{
    agent_runs as agent_runs_repo, agents as agents_repo, repositories as repositories_repo, reviews as reviews_repo,
    test_runs as test_runs_repo, workspaces as workspaces_repo,
};
use crate::error::{AppError, AppResult};
use crate::git::{ConflictedFile, MergeOutcome};
use crate::state::AppState;

/// Tri-state for a readiness signal that can genuinely never have run yet —
/// distinct from a signal that ran and failed. Mirrors `src/types/db.ts`'s
/// `ReadinessCheck`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCheck {
    NotRun,
    Passed,
    Failed,
}

/// A real merge-readiness checklist for one agent run's task — every signal
/// computed from real state, never fabricated: `tests`/`build` are the
/// latest `test_runs` row for the project (M12) narrowed to that kind,
/// `review` is the latest `reviews` row for this run (M14), and
/// `noConflicts` is a **live** dry-run merge check (`GitService::
/// merge_conflict_dry_run`) — always `Passed`/`Failed`, never `NotRun`,
/// since it costs nothing to actually check rather than remember. Mirrors
/// `src/types/db.ts`'s `MergeReadinessDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeReadinessDto {
    pub tests: ReadinessCheck,
    pub build: ReadinessCheck,
    pub review: ReadinessCheck,
    pub no_conflicts: ReadinessCheck,
    /// Populated (only) when `no_conflicts` is `Failed` — exactly which
    /// files the dry run found conflicting, so the UI can show them before
    /// the user ever attempts a real merge.
    pub conflicted_files: Vec<ConflictedFile>,
    /// `true` only when `no_conflicts` is `Passed` — conflicts are the one
    /// signal here that's a mechanical blocker (a merge with conflicts
    /// genuinely cannot complete without resolving them first); failing
    /// tests/build/review are shown honestly but left as the human's call,
    /// never silently forced or silently hidden.
    pub can_merge: bool,
}

/// The result of a real merge attempt (`merge_agent_run`). Mirrors
/// `src/types/db.ts`'s `MergeResultDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeResultDto {
    pub merged: bool,
    /// Non-empty exactly when `merged` is `false` — the primary checkout is
    /// left genuinely mid-merge with these files conflicted; nothing here
    /// is auto-resolved.
    pub conflicts: Vec<ConflictedFile>,
}

/// Everything a merge-related command needs about one agent run's task,
/// loaded once. `base_branch` is the branch the agent's worktree was
/// created from (M5) — what the merge lands *into*; `agent_branch` is the
/// worktree's own branch — what gets merged *from*. `primary_root` is the
/// repository's real checkout path (`repositories.root_path`), never the
/// agent's disposable worktree path.
#[derive(Debug)]
struct MergeContext {
    project_id: String,
    primary_root: PathBuf,
    base_branch: String,
    agent_branch: String,
}

fn load_merge_context(conn: &rusqlite::Connection, agent_run_id: &str) -> AppResult<MergeContext> {
    let agent_run = agent_runs_repo::get_by_id(conn, agent_run_id)?
        .ok_or_else(|| AppError::NotFound(format!("agent run {agent_run_id} not found")))?;
    let agent = agents_repo::get_by_id(conn, &agent_run.agent_id)?
        .ok_or_else(|| AppError::NotFound(format!("agent {} not found", agent_run.agent_id)))?;
    let workspace_id = agent_run
        .workspace_id
        .ok_or_else(|| AppError::InvalidInput(format!("agent run {agent_run_id} has no workspace")))?;
    let workspace = workspaces_repo::get_by_id(conn, &workspace_id)?
        .ok_or_else(|| AppError::NotFound(format!("workspace {workspace_id} not found")))?;
    let repository = repositories_repo::get_by_id(conn, &workspace.repository_id)?
        .ok_or_else(|| AppError::NotFound(format!("repository {} not found", workspace.repository_id)))?;
    let base_branch = workspace.base_branch.clone().ok_or_else(|| {
        AppError::InvalidInput(format!("agent run {agent_run_id}'s workspace has no recorded base branch to merge into"))
    })?;

    Ok(MergeContext {
        project_id: agent.project_id,
        primary_root: PathBuf::from(repository.root_path),
        base_branch,
        agent_branch: workspace.branch_name,
    })
}

fn readiness_for_test_run(conn: &rusqlite::Connection, project_id: &str, kind: TestRunKind) -> AppResult<ReadinessCheck> {
    let runs = test_runs_repo::list_for_project(conn, project_id, Some(kind))?;
    Ok(match runs.first().map(|r| r.status) {
        None => ReadinessCheck::NotRun,
        // A run still `running` hasn't concluded either way yet — reported
        // the same honest way as "never run", not guessed at.
        Some(TestRunStatus::Running) => ReadinessCheck::NotRun,
        Some(TestRunStatus::Success) => ReadinessCheck::Passed,
        Some(TestRunStatus::Failure) => ReadinessCheck::Failed,
    })
}

fn readiness_for_review(conn: &rusqlite::Connection, agent_run_id: &str) -> AppResult<ReadinessCheck> {
    let review = reviews_repo::get_latest_for_run(conn, agent_run_id)?;
    Ok(match review.map(|r| r.status) {
        None => ReadinessCheck::NotRun,
        Some(ReviewStatus::Pending) => ReadinessCheck::NotRun,
        Some(ReviewStatus::Passed) => ReadinessCheck::Passed,
        Some(ReviewStatus::Failed) => ReadinessCheck::Failed,
    })
}

/// Computes a real merge-readiness checklist for `agent_run_id`'s task — see
/// `MergeReadinessDto`'s own docs for exactly what each field means and how
/// it's derived. The `noConflicts` signal is a genuine dry-run merge attempt
/// (`GitService::merge_conflict_dry_run`) against the primary checkout,
/// immediately aborted either way — never a guess, and never leaves the
/// primary checkout in any different state than it found it.
#[tauri::command]
pub async fn get_merge_readiness(app: AppHandle, agent_run_id: String) -> Result<MergeReadinessDto, String> {
    run_blocking(move || -> AppResult<MergeReadinessDto> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;

        let ctx = load_merge_context(&conn, &agent_run_id)?;
        let tests = readiness_for_test_run(&conn, &ctx.project_id, TestRunKind::Test)?;
        let build = readiness_for_test_run(&conn, &ctx.project_id, TestRunKind::Build)?;
        let review = readiness_for_review(&conn, &agent_run_id)?;

        let dry_run = state.git_service.merge_conflict_dry_run(&ctx.primary_root, &ctx.agent_branch, &ctx.base_branch)?;
        let (no_conflicts, conflicted_files) = match dry_run {
            MergeOutcome::Clean => (ReadinessCheck::Passed, Vec::new()),
            MergeOutcome::Conflicts(files) => (ReadinessCheck::Failed, files),
        };

        Ok(MergeReadinessDto { tests, build, review, no_conflicts, conflicted_files, can_merge: no_conflicts == ReadinessCheck::Passed })
    })
    .await
}

/// Performs a **real** merge of `agent_run_id`'s worktree branch into its
/// base branch, in the repository's primary checkout. Never auto-resolves a
/// conflict: on `MergeOutcome::Conflicts`, the primary checkout is left
/// genuinely mid-merge and this returns `merged: false` with the conflicted
/// files — the caller decides what happens next (`abort_agent_run_merge`, or
/// `resolve_agent_run_merge_conflicts_with_agent`).
#[tauri::command]
pub async fn merge_agent_run(app: AppHandle, agent_run_id: String) -> Result<MergeResultDto, String> {
    run_blocking(move || -> AppResult<MergeResultDto> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let ctx = load_merge_context(&conn, &agent_run_id)?;

        let outcome = state.git_service.merge_branch(&ctx.primary_root, &ctx.agent_branch, &ctx.base_branch)?;
        Ok(match outcome {
            MergeOutcome::Clean => MergeResultDto { merged: true, conflicts: Vec::new() },
            MergeOutcome::Conflicts(files) => MergeResultDto { merged: false, conflicts: files },
        })
    })
    .await
}

/// The honest "back out" path: `git merge --abort` in the primary checkout,
/// for a conflicted merge the user doesn't want to resolve (manually or via
/// the AI resolver). Always available — never gated on an API key, unlike
/// the AI resolver.
#[tauri::command]
pub async fn abort_agent_run_merge(app: AppHandle, agent_run_id: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let ctx = load_merge_context(&conn, &agent_run_id)?;
        state.git_service.abort_merge(&ctx.primary_root)
    })
    .await
}

/// Explicit, user-requested fallback: runs the bounded AI conflict resolver
/// (`agent::conflict_resolver::resolve_conflicts_with_agent`) against
/// `agent_run_id`'s currently-conflicted primary-checkout merge. Never
/// triggered automatically by `merge_agent_run` itself — see that module's
/// own docs for the full safety story (scoped tool list, iteration cap, and
/// the real check before it will ever commit).
#[tauri::command]
pub async fn resolve_agent_run_merge_conflicts_with_agent(app: AppHandle, agent_run_id: String) -> Result<(), String> {
    conflict_resolver::resolve_conflicts_with_agent(&app, &agent_run_id).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use rusqlite::Connection;

    fn migrated_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test')", []).unwrap();
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", []).unwrap();
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", []).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .unwrap();
        conn
    }

    // -- readiness_for_test_run ----------------------------------------------

    #[test]
    fn readiness_for_test_run_is_not_run_when_no_row_exists() {
        let conn = migrated_conn();
        assert_eq!(readiness_for_test_run(&conn, "p1", TestRunKind::Test).unwrap(), ReadinessCheck::NotRun);
    }

    #[test]
    fn readiness_for_test_run_reflects_the_most_recent_rows_status() {
        let conn = migrated_conn();
        let first = test_runs_repo::insert_running(&conn, "p1", TestRunKind::Test, "npm test").unwrap();
        test_runs_repo::complete(&conn, &first.id, TestRunStatus::Failure, "boom", Some(1)).unwrap();
        assert_eq!(readiness_for_test_run(&conn, "p1", TestRunKind::Test).unwrap(), ReadinessCheck::Failed);

        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = test_runs_repo::insert_running(&conn, "p1", TestRunKind::Test, "npm test").unwrap();
        test_runs_repo::complete(&conn, &second.id, TestRunStatus::Success, "ok", Some(0)).unwrap();
        assert_eq!(readiness_for_test_run(&conn, "p1", TestRunKind::Test).unwrap(), ReadinessCheck::Passed, "must reflect the latest row, not the first");
    }

    #[test]
    fn readiness_for_test_run_is_not_run_while_still_running() {
        let conn = migrated_conn();
        test_runs_repo::insert_running(&conn, "p1", TestRunKind::Build, "cargo build").unwrap();
        assert_eq!(
            readiness_for_test_run(&conn, "p1", TestRunKind::Build).unwrap(),
            ReadinessCheck::NotRun,
            "an in-progress run hasn't concluded either way yet"
        );
    }

    #[test]
    fn readiness_for_test_run_is_scoped_to_the_requested_kind() {
        let conn = migrated_conn();
        let build = test_runs_repo::insert_running(&conn, "p1", TestRunKind::Build, "cargo build").unwrap();
        test_runs_repo::complete(&conn, &build.id, TestRunStatus::Success, "ok", Some(0)).unwrap();
        // No `Test`-kind row was ever inserted.
        assert_eq!(readiness_for_test_run(&conn, "p1", TestRunKind::Test).unwrap(), ReadinessCheck::NotRun);
        assert_eq!(readiness_for_test_run(&conn, "p1", TestRunKind::Build).unwrap(), ReadinessCheck::Passed);
    }

    // -- readiness_for_review -------------------------------------------------

    #[test]
    fn readiness_for_review_is_not_run_when_never_requested() {
        let conn = migrated_conn();
        assert_eq!(readiness_for_review(&conn, "run1").unwrap(), ReadinessCheck::NotRun);
    }

    #[test]
    fn readiness_for_review_is_not_run_while_pending_not_failed() {
        let conn = migrated_conn();
        reviews_repo::insert_pending(&conn, "run1").unwrap();
        assert_eq!(
            readiness_for_review(&conn, "run1").unwrap(),
            ReadinessCheck::NotRun,
            "a review still in progress is not yet a 'failed' review"
        );
    }

    #[test]
    fn readiness_for_review_reflects_passed_and_failed() {
        let conn = migrated_conn();
        let review = reviews_repo::insert_pending(&conn, "run1").unwrap();
        reviews_repo::complete(&conn, &review.id, 90, "[]", ReviewStatus::Passed).unwrap();
        assert_eq!(readiness_for_review(&conn, "run1").unwrap(), ReadinessCheck::Passed);

        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = reviews_repo::insert_pending(&conn, "run1").unwrap();
        reviews_repo::complete(&conn, &second.id, 10, "[]", ReviewStatus::Failed).unwrap();
        assert_eq!(readiness_for_review(&conn, "run1").unwrap(), ReadinessCheck::Failed, "must reflect the latest review");
    }

    // -- load_merge_context ---------------------------------------------------

    #[test]
    fn load_merge_context_resolves_primary_root_base_and_agent_branch() {
        let conn = migrated_conn();
        conn.execute(
            "INSERT INTO workspaces (id, repository_id, agent_run_id, kind, path, branch_name, base_branch, status, created_at) \
             VALUES ('w1', 'r1', 'run1', 'agent', '/tmp/r1/.forge-workspace/worktrees/run1', 'forge/agent/bot/run1', 'main', 'active', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute("UPDATE agent_runs SET workspace_id = 'w1' WHERE id = 'run1'", []).unwrap();

        let ctx = load_merge_context(&conn, "run1").expect("load_merge_context");
        assert_eq!(ctx.project_id, "p1");
        assert_eq!(ctx.primary_root, PathBuf::from("/tmp/r1"), "must resolve to the repository's root, never the agent's worktree path");
        assert_eq!(ctx.base_branch, "main");
        assert_eq!(ctx.agent_branch, "forge/agent/bot/run1");
    }

    #[test]
    fn load_merge_context_errors_clearly_when_the_run_has_no_workspace_yet() {
        let conn = migrated_conn();
        let err = load_merge_context(&conn, "run1").expect_err("a run with no workspace can't be merged");
        assert!(err.to_string().contains("no workspace"));
    }

    #[test]
    fn load_merge_context_errors_clearly_when_base_branch_was_never_recorded() {
        let conn = migrated_conn();
        conn.execute(
            "INSERT INTO workspaces (id, repository_id, agent_run_id, kind, path, branch_name, base_branch, status, created_at) \
             VALUES ('w1', 'r1', 'run1', 'agent', '/tmp/r1/.forge-workspace/worktrees/run1', 'forge/agent/bot/run1', NULL, 'active', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute("UPDATE agent_runs SET workspace_id = 'w1' WHERE id = 'run1'", []).unwrap();

        let err = load_merge_context(&conn, "run1").expect_err("no base branch recorded");
        assert!(err.to_string().contains("base branch"));
    }
}

//! M12: per-project test/lint/build command settings (detected + editable)
//! and manual test/lint/build runs — the backend for the Testing tab.
//!
//! Command *settings* reuse the same generic `settings` key/value store
//! every other per-project setting goes through (see
//! `project_detect::project_setting_key`); the only thing new here is a
//! companion `.source` key recording whether a value was auto-detected or
//! set by the user, so the UI can show an honest three-state badge instead
//! of just a maybe-empty text field. Manual *runs* reuse
//! `agent::process::run_shell_command` — the exact same spawn/timeout/cancel
//! logic `agent::tools`'s `run_tests`/`run_linter`/`run_build` tools already
//! use — rather than re-implementing process execution here.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::agent::process::{run_shell_command, ProcessOutcome};
use crate::commands::run_blocking;
use crate::db::models::{TestRun, TestRunKind, TestRunStatus};
use crate::db::repository::{repositories as repositories_repo, settings as settings_repo, test_runs as test_runs_repo};
use crate::error::{AppError, AppResult};
use crate::project_detect::{project_setting_key, project_setting_source_key};
use crate::state::AppState;

/// A manual test/lint/build run has no natural per-run timeout the way an
/// agent's tool calls do (`agent.tool_timeout_ms`, defaulted to 30s in
/// `agent::tool_loop` — far too short for a real test suite) — this is
/// deliberately its own, much longer default.
const DEFAULT_TEST_RUN_TIMEOUT_MS: u64 = 5 * 60 * 1000;

fn kind_from_str(kind: &str) -> Result<TestRunKind, String> {
    match kind {
        "test" => Ok(TestRunKind::Test),
        "lint" => Ok(TestRunKind::Lint),
        "build" => Ok(TestRunKind::Build),
        other => Err(format!("unknown kind '{other}' — expected 'test', 'lint', or 'build'")),
    }
}

fn kind_setting_name(kind: &str) -> String {
    format!("{kind}_command")
}

/// One command's configured value plus its provenance, for the Testing
/// tab's "detected" vs. "you set this" vs. "not configured" badge.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSettingDto {
    pub value: Option<String>,
    /// `"detected"` | `"user"` | `null` (never set, or set before M12 and
    /// so has no recorded source — treated the same as `null`: honestly
    /// "unknown provenance", not silently relabeled as either).
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCommandSettingsDto {
    pub test_command: CommandSettingDto,
    pub lint_command: CommandSettingDto,
    pub build_command: CommandSettingDto,
}

fn read_command_setting(conn: &rusqlite::Connection, project_id: &str, kind: &str) -> AppResult<CommandSettingDto> {
    let setting_name = kind_setting_name(kind);
    let value = settings_repo::get(conn, &project_setting_key(project_id, &setting_name))?.map(|s| s.value);
    let source = settings_repo::get(conn, &project_setting_source_key(project_id, &setting_name))?.map(|s| s.value);
    Ok(CommandSettingDto { value, source })
}

/// The detected/configured test/lint/build commands for `project_id`, each
/// labeled with whether it was auto-detected, user-set, or never configured.
#[tauri::command]
pub async fn get_project_command_settings(app: AppHandle, project_id: String) -> Result<ProjectCommandSettingsDto, String> {
    run_blocking(move || -> AppResult<ProjectCommandSettingsDto> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        Ok(ProjectCommandSettingsDto {
            test_command: read_command_setting(&conn, &project_id, "test")?,
            lint_command: read_command_setting(&conn, &project_id, "lint")?,
            build_command: read_command_setting(&conn, &project_id, "build")?,
        })
    })
    .await
}

/// Sets `project_id`'s `kind` command (`"test"`/`"lint"`/`"build"`) to
/// `value` and marks its source as `"user"` — the Testing tab's edit path,
/// distinct from the M12 auto-detection prefill so the UI never confuses
/// "you typed this" with "we guessed this from your files".
#[tauri::command]
pub async fn set_project_command_setting(app: AppHandle, project_id: String, kind: String, value: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let kind = kind_from_str(&kind).map_err(AppError::InvalidInput)?;
        let setting_name = kind_setting_name(match kind {
            TestRunKind::Test => "test",
            TestRunKind::Lint => "lint",
            TestRunKind::Build => "build",
        });
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput("command cannot be empty".to_string()));
        }
        settings_repo::set(&conn, &project_setting_key(&project_id, &setting_name), trimmed)?;
        settings_repo::set(&conn, &project_setting_source_key(&project_id, &setting_name), "user")?;
        Ok(())
    })
    .await
}

/// Runs `project_id`'s configured `kind` command (`"test"`/`"lint"`/
/// `"build"`) in the project's primary repository root — independently of
/// any agent run, so the user can trigger it manually from the Testing tab.
/// Errors clearly (never guesses a command) if none is configured, exactly
/// like `agent::tools::run_tests`/`run_linter`/`run_build` already do for an
/// agent run. Records a `test_runs` row for the full run so it shows up in
/// the tab's history, not just as a one-shot result.
#[tauri::command]
pub async fn run_test_suite(app: AppHandle, project_id: String, kind: String) -> Result<TestRun, String> {
    let run_kind = kind_from_str(&kind)?;
    let setting_name = kind_setting_name(&kind);

    let (command, repo_root) = run_blocking({
        let app = app.clone();
        let project_id = project_id.clone();
        move || -> AppResult<(String, PathBuf)> {
            let state = app.state::<AppState>();
            let conn = state.db.get()?;
            let repository = repositories_repo::get_by_project_id(&conn, &project_id)?
                .ok_or_else(|| AppError::NotFound(format!("project {project_id} has no repository")))?;
            let command = settings_repo::get(&conn, &project_setting_key(&project_id, &setting_name))?
                .map(|s| s.value)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| {
                    AppError::InvalidInput(format!(
                        "no {kind} command configured for this project — set it in the Testing tab first"
                    ))
                })?;
            Ok((command, PathBuf::from(repository.root_path)))
        }
    })
    .await?;

    let running = run_blocking({
        let app = app.clone();
        let project_id = project_id.clone();
        let command = command.clone();
        move || -> AppResult<TestRun> {
            let state = app.state::<AppState>();
            let conn = state.db.get()?;
            test_runs_repo::insert_running(&conn, &project_id, run_kind, &command)
        }
    })
    .await?;

    // A fresh, owned adapter rather than borrowing `AppState.os_adapter`
    // (behind a short-lived `State` guard) across this `.await` — the same
    // approach `agent::tool_loop::run_agent_loop_inner` uses for the same
    // reason. `OperatingSystemAdapter` is stateless platform behavior, so a
    // new instance is equivalent to the shared one.
    let os_adapter = crate::os_adapter::current();
    let outcome = run_shell_command(
        os_adapter.as_ref(),
        &repo_root,
        &command,
        Duration::from_millis(DEFAULT_TEST_RUN_TIMEOUT_MS),
        CancellationToken::new(),
    )
    .await;

    let (status, output, exit_code) = match outcome {
        ProcessOutcome::Finished { stdout, stderr, exit_code, success } => {
            let mut text = String::new();
            if !stdout.is_empty() {
                text.push_str(&format!("stdout:\n{stdout}\n"));
            }
            if !stderr.is_empty() {
                text.push_str(&format!("stderr:\n{stderr}\n"));
            }
            if text.is_empty() {
                text.push_str("(no output)");
            }
            (if success { TestRunStatus::Success } else { TestRunStatus::Failure }, text, exit_code.map(i64::from))
        }
        ProcessOutcome::TimedOut { timeout_ms } => {
            (TestRunStatus::Failure, format!("command timed out after {timeout_ms}ms and was killed"), None)
        }
        ProcessOutcome::Cancelled => (TestRunStatus::Failure, "command was cancelled".to_string(), None),
        ProcessOutcome::SpawnFailed(message) => (TestRunStatus::Failure, message, None),
    };

    let run_id = running.id.clone();
    run_blocking(move || -> AppResult<TestRun> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        test_runs_repo::complete(&conn, &run_id, status, &output, exit_code)?;
        test_runs_repo::get_by_id(&conn, &run_id)?
            .ok_or_else(|| AppError::NotFound(format!("test run {run_id} vanished after completion")))
    })
    .await
}

/// `project_id`'s run history, most recent first, optionally narrowed to one
/// `kind`.
#[tauri::command]
pub async fn list_test_runs(app: AppHandle, project_id: String, kind: Option<String>) -> Result<Vec<TestRun>, String> {
    run_blocking(move || -> AppResult<Vec<TestRun>> {
        let kind = kind.map(|k| kind_from_str(&k)).transpose().map_err(AppError::InvalidInput)?;
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        test_runs_repo::list_for_project(&conn, &project_id, kind)
    })
    .await
}

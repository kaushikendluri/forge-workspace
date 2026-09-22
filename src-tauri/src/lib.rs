//! Library crate for forge-workspace's Rust backend. `main.rs` is a thin
//! entry point that calls `run()`.
//!
//! Module map:
//! - `agent`    — the Anthropic tool-calling run loop (M6)
//! - `browser`  — M16: real browser automation (`chromiumoxide`) behind a testable trait, for the agent's browser_* tools
//! - `commands` — `#[tauri::command]` functions exposed to the frontend
//! - `db`       — SQLite pool, migrations, row models, per-table repositories
//! - `os_adapter` — platform-specific shell/PATH/env behavior
//! - `git`      — git CLI wrapper (worktrees, status, diff)
//! - `project_detect` — M12: detects default test/lint/build commands from a repo's own files
//! - `orchestrator` — mission planning (M8: objective -> structured, human-approved task plan) and
//!   execution (M9: the scheduler walks an approved mission's task graph and runs it)
//! - `terminal` — PTY-backed terminal sessions
//! - `secrets`  — OS keychain-backed API key storage
//! - `events`   — typed event emission helpers
//! - `state`    — shared `AppState` (db pool, terminal registry, active agent runs)
//! - `error`    — the app-wide `AppError`/`AppResult` types

pub mod agent;
pub mod browser;
pub mod commands;
pub mod db;
pub mod error;
pub mod events;
pub mod git;
pub mod orchestrator;
pub mod os_adapter;
pub mod project_detect;
pub mod secrets;
pub mod state;
pub mod terminal;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

use browser::{BrowserManager, ChromiumoxideBrowserBackend};
use git::GitCliService;
use state::AppState;

/// Resolves where the SQLite database file lives, inside Tauri's per-app
/// data directory (e.g. `%APPDATA%/com.forgeworkspace.app` on Windows,
/// `~/Library/Application Support/com.forgeworkspace.app` on macOS).
fn resolve_db_path(app: &tauri::AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_data_dir()
        .expect("app data dir should be resolvable");
    dir.join("forge-workspace.db")
}

/// M16: where `agent::tools::browser_screenshot_tool` saves real PNG
/// screenshots — a `screenshots` subdirectory alongside the SQLite database
/// itself (see [`resolve_db_path`]), inside the app's own per-app data
/// directory. Screenshots are real files referenced by path from
/// `visual_snapshots` rows, never inlined as base64 blobs in SQLite.
fn resolve_screenshots_dir(app: &tauri::AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_data_dir()
        .expect("app data dir should be resolvable");
    dir.join("screenshots")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let db_path = resolve_db_path(&app.handle());
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let pool = db::create_pool(&db_path)?;
            {
                let mut conn = pool.get()?;
                db::migrations::run_migrations(&mut conn)?;
            }

            let os_adapter = os_adapter::current();
            let git_service: Box<dyn git::GitService> = Box::new(GitCliService::new(os_adapter.as_ref()));

            let screenshots_dir = resolve_screenshots_dir(&app.handle());
            std::fs::create_dir_all(&screenshots_dir)?;
            // A fresh, independent adapter instance (cheap — see
            // `agent::tool_loop`'s own comment on doing the same), since
            // `os_adapter` above is about to be moved into `AppState::new`.
            let browser_manager =
                Arc::new(BrowserManager::new(Arc::new(ChromiumoxideBrowserBackend::new(os_adapter::current()))));

            app.manage(AppState::new(pool, os_adapter, git_service, browser_manager, screenshots_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::project_commands::open_project,
            commands::project_commands::init_project,
            commands::project_commands::list_projects,
            commands::git_commands::git_status,
            commands::git_commands::git_branches,
            commands::git_commands::git_current_branch,
            commands::git_commands::git_diff_file,
            commands::git_commands::git_log,
            commands::fs_commands::list_directory,
            commands::fs_commands::read_file_preview,
            commands::terminal_commands::terminal_spawn,
            commands::terminal_commands::terminal_write,
            commands::terminal_commands::terminal_resize,
            commands::terminal_commands::terminal_kill,
            commands::settings_commands::get_setting,
            commands::settings_commands::set_setting,
            commands::settings_commands::list_settings,
            commands::settings_commands::set_api_key,
            commands::settings_commands::has_api_key,
            commands::settings_commands::clear_api_key,
            commands::settings_commands::list_model_configs,
            commands::notifications_commands::list_notifications,
            commands::notifications_commands::mark_notification_read,
            commands::notifications_commands::unread_notification_count,
            commands::agent_commands::create_agent,
            commands::agent_commands::list_agents,
            commands::agent_commands::start_worktree_for_agent,
            commands::agent_commands::remove_agent_workspace,
            commands::agent_run_commands::start_agent_run,
            commands::agent_run_commands::stop_agent_run,
            commands::agent_run_commands::get_agent_run,
            commands::agent_run_commands::list_agent_runs,
            commands::agent_run_commands::list_tool_calls,
            commands::agent_run_commands::list_activity_events,
            commands::agent_run_commands::get_run_diff,
            commands::mission_commands::create_mission,
            commands::mission_commands::approve_mission_plan,
            commands::mission_commands::get_mission,
            commands::mission_commands::list_missions,
            commands::mission_commands::list_mission_tasks,
            commands::mission_commands::list_mission_board,
            commands::mission_commands::list_agent_messages,
            commands::mission_commands::start_mission,
            commands::mission_commands::stop_mission,
            commands::testing_commands::get_project_command_settings,
            commands::testing_commands::set_project_command_setting,
            commands::testing_commands::run_test_suite,
            commands::testing_commands::list_test_runs,
            commands::review_commands::request_review,
            commands::review_commands::get_review,
            commands::merge_commands::get_merge_readiness,
            commands::merge_commands::merge_agent_run,
            commands::merge_commands::abort_agent_run_merge,
            commands::merge_commands::resolve_agent_run_merge_conflicts_with_agent,
            commands::visual_commands::list_visual_snapshots,
            commands::visual_commands::get_visual_snapshot_image,
            commands::visual_commands::accept_visual_snapshot,
            commands::visual_commands::flag_visual_snapshot,
            commands::visual_commands::create_visual_regression_follow_up_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running forge-workspace");
}

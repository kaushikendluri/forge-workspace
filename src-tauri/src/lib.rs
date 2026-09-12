//! Library crate for forge-workspace's Rust backend. `main.rs` is a thin
//! entry point that calls `run()`.
//!
//! Module map:
//! - `commands` — `#[tauri::command]` functions exposed to the frontend
//! - `db`       — SQLite pool, migrations, row models, per-table repositories
//! - `os_adapter` — platform-specific shell/PATH/env behavior
//! - `git`      — git CLI wrapper (worktrees, status, diff)
//! - `terminal` — PTY-backed terminal sessions
//! - `secrets`  — OS keychain-backed API key storage
//! - `events`   — typed event emission helpers
//! - `state`    — shared `AppState` (db pool, terminal registry)
//! - `error`    — the app-wide `AppError`/`AppResult` types

pub mod commands;
pub mod db;
pub mod error;
pub mod events;
pub mod git;
pub mod os_adapter;
pub mod secrets;
pub mod state;
pub mod terminal;

use std::path::PathBuf;

use tauri::Manager;

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

            app.manage(AppState::new(pool, os_adapter, git_service));
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running forge-workspace");
}

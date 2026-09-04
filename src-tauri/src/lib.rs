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
        .setup(|app| {
            let db_path = resolve_db_path(&app.handle());
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let pool = db::create_pool(&db_path)?;
            {
                let conn = pool.get()?;
                db::migrations::run_migrations(&conn)?;
            }

            app.manage(AppState::new(pool));
            Ok(())
        })
        // TODO(M2+): register commands as they're implemented, e.g.:
        // .invoke_handler(tauri::generate_handler![
        //     commands::project_commands::list_projects,
        //     commands::project_commands::get_project,
        //     ...
        // ])
        .run(tauri::generate_context!())
        .expect("error while running forge-workspace");
}

//! Commands backing `src/components/terminal/TerminalView.tsx` /
//! `src/stores/useTerminalStore.ts`. Output is pushed the other direction
//! via the `terminal:output` / `terminal:exit` events (see `events/mod.rs`
//! and `terminal/mod.rs`), not returned from these commands.

use tauri::{AppHandle, Manager};

use crate::commands::run_blocking;
use crate::state::AppState;

/// Spawns a new PTY-backed shell session rooted at `cwd` and returns its id.
#[tauri::command]
pub async fn terminal_spawn(app: AppHandle, cwd: String) -> Result<String, String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.terminals.spawn(&app, state.os_adapter.as_ref(), &cwd)
    })
    .await
}

/// Writes `data` (raw keystrokes from the frontend's xterm.js instance) to
/// the session's pty stdin.
#[tauri::command]
pub async fn terminal_write(app: AppHandle, terminal_id: String, data: String) -> Result<(), String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.terminals.write(&terminal_id, data.as_bytes())
    })
    .await
}

/// Resizes the session's pty to match the frontend terminal's new
/// dimensions.
#[tauri::command]
pub async fn terminal_resize(app: AppHandle, terminal_id: String, cols: u16, rows: u16) -> Result<(), String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.terminals.resize(&terminal_id, cols, rows)
    })
    .await
}

/// Kills the session's child process and drops its pty handles.
#[tauri::command]
pub async fn terminal_kill(app: AppHandle, terminal_id: String) -> Result<(), String> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        state.terminals.kill(&terminal_id)
    })
    .await
}

//! Commands backing `src/components/terminal/TerminalView.tsx` /
//! `src/stores/useTerminalStore.ts`. Output is pushed the other direction
//! via the `terminal:output` event (see `events/mod.rs`), not returned
//! from these commands.

use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn open_terminal(_app: AppHandle, _state: State<AppState>, _project_id: String, _cwd: String) -> AppResult<String> {
    todo!("M3: terminal::spawn_session, return the new session id")
}

#[tauri::command]
pub fn write_terminal(_state: State<AppState>, _session_id: String, _data: String) -> AppResult<()> {
    todo!("M3: terminal::write_to_session")
}

#[tauri::command]
pub fn resize_terminal(_state: State<AppState>, _session_id: String, _cols: u16, _rows: u16) -> AppResult<()> {
    todo!("M3: terminal::resize_session")
}

#[tauri::command]
pub fn close_terminal(_state: State<AppState>, _session_id: String) -> AppResult<()> {
    todo!("M3: terminal::kill_session")
}

//! `#[tauri::command]` functions, grouped by domain and registered in
//! `lib.rs`'s `tauri::generate_handler!` once implemented. Command names
//! here correspond 1:1 to `src/lib/tauri.ts`'s `CommandMap` keys.

pub mod agent_commands;
pub mod agent_run_commands;
pub mod fs_commands;
pub mod git_commands;
pub mod mission_commands;
pub mod notifications_commands;
pub mod project_commands;
pub mod settings_commands;
pub mod terminal_commands;
pub mod testing_commands;

use crate::error::AppResult;

/// Runs a blocking closure (DB/git calls, neither of which is async) on
/// Tauri's blocking thread pool, then flattens the join error and the
/// closure's own `AppError` into the plain `Result<T, String>` every
/// `#[tauri::command]` here returns — `AppError` already renders a clear
/// message via its `Display` impl, and Tauri's IPC only needs `Serialize`
/// on the error type, which `String` trivially satisfies.
pub(crate) async fn run_blocking<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("background task failed: {e}"))
        .and_then(|r| r.map_err(|e| e.to_string()))
}

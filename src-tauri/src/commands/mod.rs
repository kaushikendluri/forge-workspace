//! `#[tauri::command]` functions, grouped by domain and registered in
//! `lib.rs`'s `tauri::generate_handler!` once implemented. Command names
//! here correspond 1:1 to `src/lib/tauri.ts`'s `CommandMap` keys.

pub mod fs_commands;
pub mod git_commands;
pub mod project_commands;
pub mod settings_commands;
pub mod terminal_commands;

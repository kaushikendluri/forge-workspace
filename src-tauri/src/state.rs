//! Shared application state, managed by Tauri and injected into commands
//! via `tauri::State<AppState>`.

use std::sync::Mutex;

use crate::db::DbPool;
use crate::terminal::TerminalRegistry;

pub struct AppState {
    pub db: DbPool,
    pub terminals: Mutex<TerminalRegistry>,
}

impl AppState {
    pub fn new(db: DbPool) -> Self {
        Self {
            db,
            terminals: Mutex::new(TerminalRegistry::new()),
        }
    }
}

//! Shared application state, managed by Tauri and injected into commands
//! via `tauri::State<AppState>` / `AppHandle::state::<AppState>()`.

use std::sync::Mutex;

use crate::db::DbPool;
use crate::git::GitService;
use crate::os_adapter::OperatingSystemAdapter;
use crate::terminal::TerminalRegistry;

pub struct AppState {
    pub db: DbPool,
    pub os_adapter: Box<dyn OperatingSystemAdapter>,
    pub git_service: Box<dyn GitService>,
    pub terminals: Mutex<TerminalRegistry>,
}

impl AppState {
    pub fn new(
        db: DbPool,
        os_adapter: Box<dyn OperatingSystemAdapter>,
        git_service: Box<dyn GitService>,
    ) -> Self {
        Self {
            db,
            os_adapter,
            git_service,
            terminals: Mutex::new(TerminalRegistry::new()),
        }
    }
}

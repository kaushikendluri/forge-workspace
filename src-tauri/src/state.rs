//! Shared application state, managed by Tauri and injected into commands
//! via `tauri::State<AppState>` / `AppHandle::state::<AppState>()`.

use crate::db::DbPool;
use crate::git::GitService;
use crate::os_adapter::OperatingSystemAdapter;
use crate::terminal::TerminalManager;

pub struct AppState {
    pub db: DbPool,
    pub os_adapter: Box<dyn OperatingSystemAdapter>,
    pub git_service: Box<dyn GitService>,
    pub terminals: TerminalManager,
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
            terminals: TerminalManager::new(),
        }
    }
}

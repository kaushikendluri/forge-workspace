//! Shared application state, managed by Tauri and injected into commands
//! via `tauri::State<AppState>` / `AppHandle::state::<AppState>()`.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use crate::db::DbPool;
use crate::git::GitService;
use crate::os_adapter::OperatingSystemAdapter;
use crate::terminal::TerminalManager;

pub struct AppState {
    pub db: DbPool,
    pub os_adapter: Box<dyn OperatingSystemAdapter>,
    pub git_service: Box<dyn GitService>,
    pub terminals: TerminalManager,
    /// Cancellation tokens for every agent run currently executing
    /// (`agent::tool_loop::run_agent_loop`), keyed by `agent_runs.id`.
    /// `start_agent_run` inserts an entry before spawning the loop;
    /// `stop_agent_run` looks the token up and fires it; the loop itself
    /// removes its own entry on every exit path (see
    /// `agent::tool_loop::ActiveRunGuard`). A run absent from this map is
    /// simply not currently executing — not an error condition.
    pub active_runs: Mutex<HashMap<String, CancellationToken>>,
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
            active_runs: Mutex::new(HashMap::new()),
        }
    }
}

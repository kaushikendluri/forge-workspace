//! Shared application state, managed by Tauri and injected into commands
//! via `tauri::State<AppState>` / `AppHandle::state::<AppState>()`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::browser::BrowserManager;
use crate::db::DbPool;
use crate::git::GitService;
use crate::os_adapter::OperatingSystemAdapter;
use crate::terminal::TerminalManager;

pub struct AppState {
    pub db: DbPool,
    pub os_adapter: Box<dyn OperatingSystemAdapter>,
    pub git_service: Box<dyn GitService>,
    pub terminals: TerminalManager,
    /// M16: the per-agent-run browser session registry — see
    /// `browser::BrowserManager`'s own docs.
    pub browser_manager: Arc<BrowserManager>,
    /// M16: where `agent::tools::browser_screenshot_tool` saves real PNG
    /// screenshots, resolved once alongside `resolve_db_path` in `lib.rs`'s
    /// `setup` — inside the app's own per-app data directory, never inside
    /// any agent's worktree.
    pub screenshots_dir: PathBuf,
    /// Cancellation tokens for every agent run currently executing
    /// (`agent::tool_loop::run_agent_loop`), keyed by `agent_runs.id`.
    /// `start_agent_run` inserts an entry before spawning the loop;
    /// `stop_agent_run` looks the token up and fires it; the loop itself
    /// removes its own entry on every exit path (see
    /// `agent::tool_loop::ActiveRunGuard`). A run absent from this map is
    /// simply not currently executing — not an error condition.
    pub active_runs: Mutex<HashMap<String, CancellationToken>>,
    /// Cancellation tokens for every mission currently being executed by
    /// `orchestrator::scheduler::run_mission`, keyed by `missions.id` — the
    /// same shape as `active_runs`, one level up. `start_mission` inserts an
    /// entry before spawning the scheduler loop; `stop_mission` looks the
    /// token up and fires it. The scheduler (M10) may have several tasks
    /// running concurrently for one mission (each with its own entry in
    /// this mission's *own* `active_runs`, not tracked separately here) —
    /// every one of them holds a `.clone()` of this same token and
    /// independently forwards it as a real `stop_agent_run` call against its
    /// own run, so firing this one token here stops all of them, not just
    /// one. The scheduler removes its own entry on every exit path.
    pub active_missions: Mutex<HashMap<String, CancellationToken>>,
}

impl AppState {
    pub fn new(
        db: DbPool,
        os_adapter: Box<dyn OperatingSystemAdapter>,
        git_service: Box<dyn GitService>,
        browser_manager: Arc<BrowserManager>,
        screenshots_dir: PathBuf,
    ) -> Self {
        Self {
            db,
            os_adapter,
            git_service,
            terminals: TerminalManager::new(),
            active_runs: Mutex::new(HashMap::new()),
            active_missions: Mutex::new(HashMap::new()),
            browser_manager,
            screenshots_dir,
        }
    }
}

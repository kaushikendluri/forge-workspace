//! PTY-backed terminal sessions for the Terminal tab, built on
//! `portable-pty`. Output streams to the frontend via the
//! `terminal:output` / `terminal:exit` events defined in
//! `src/types/events.ts`; input arrives through a `terminal_commands.rs`
//! command that writes to the PTY's writer half.

use std::collections::HashMap;

use crate::error::AppResult;

pub struct TerminalSession {
    pub id: String,
    pub project_id: String,
    // TODO(M3): hold the portable_pty::MasterPty + Child handle here once
    // sessions are actually spawned.
}

/// Registry of live terminal sessions, held in `AppState` behind a mutex.
#[derive(Default)]
pub struct TerminalRegistry {
    sessions: HashMap<String, TerminalSession>,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self { sessions: HashMap::new() }
    }

    pub fn get(&self, id: &str) -> Option<&TerminalSession> {
        self.sessions.get(id)
    }
}

/// Spawns a new PTY running the platform default shell
/// (`os_adapter::OperatingSystemAdapter::shell_invocation`) rooted at
/// `cwd`, and registers it under a fresh id.
pub fn spawn_session(_project_id: &str, _cwd: &str) -> AppResult<TerminalSession> {
    todo!("M3: portable_pty::native_pty_system().openpty(...) + spawn_command")
}

/// Writes `data` (raw bytes from the frontend's xterm.js instance) to the
/// PTY's stdin.
pub fn write_to_session(_session_id: &str, _data: &[u8]) -> AppResult<()> {
    todo!("M3: write to the PTY writer half")
}

/// Resizes the PTY to match the frontend terminal's new dimensions.
pub fn resize_session(_session_id: &str, _cols: u16, _rows: u16) -> AppResult<()> {
    todo!("M3: MasterPty::resize")
}

pub fn kill_session(_session_id: &str) -> AppResult<()> {
    todo!("M3: kill the child process and drop the PTY handles")
}

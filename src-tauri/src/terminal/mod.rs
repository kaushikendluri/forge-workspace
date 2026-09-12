//! PTY-backed terminal sessions for the Terminal tab, built on
//! `portable-pty`. Output streams to the frontend via the
//! `terminal:output` / `terminal:exit` events defined in
//! `src/types/events.ts` (payload carries `terminalId` so one listener can
//! serve every open terminal tab); input arrives through
//! `commands::terminal_commands`, which writes to the PTY's writer half.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize, PtySystem, SlavePty};
use serde::Serialize;
use tauri::AppHandle;

use crate::error::{AppError, AppResult};
use crate::events;
use crate::os_adapter::OperatingSystemAdapter;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalOutputPayload {
    terminal_id: String,
    /// Base64-encoded raw PTY output bytes (may not be valid UTF-8 —
    /// terminal output includes control sequences and arbitrary program
    /// output, not just text).
    chunk: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalExitPayload {
    terminal_id: String,
    exit_code: Option<i32>,
}

/// One live PTY-backed shell session. The reader half is handed to a
/// background thread at spawn time rather than stored here.
struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

/// Registry of live terminal sessions, held in `AppState`. Cheap to clone
/// (an `Arc` around the actual map) so it can be handed into the background
/// reader thread each `spawn` starts.
#[derive(Clone, Default)]
pub struct TerminalManager {
    sessions: Arc<Mutex<HashMap<String, TerminalSession>>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns a new PTY running the platform's default interactive shell
    /// (`os_adapter.shell_invocation()`) rooted at `cwd`, wires a background
    /// thread to forward its output as `terminal:output` events, and
    /// returns the new session's id.
    pub fn spawn(&self, app: &AppHandle, os_adapter: &dyn OperatingSystemAdapter, cwd: &str) -> AppResult<String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| AppError::Other(format!("failed to open a pty: {e}")))?;

        let invocation = os_adapter.shell_invocation();
        let (program, args) = invocation
            .split_first()
            .ok_or_else(|| AppError::Other("no shell invocation configured for this platform".to_string()))?;

        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        if !cwd.is_empty() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| AppError::Other(format!("failed to spawn shell '{program}': {e}")))?;
        // The slave side belongs to the child now; dropping our copy is
        // what lets the master reader see EOF once the child exits (an open
        // slave fd in this process would otherwise keep the pty "readable
        // forever" on Unix).
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| AppError::Other(format!("failed to open pty reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| AppError::Other(format!("failed to open pty writer: {e}")))?;

        let id = uuid::Uuid::new_v4().to_string();
        let session = TerminalSession { master: pair.master, writer, child };
        self.sessions
            .lock()
            .map_err(|_| AppError::Other("terminal registry lock poisoned".to_string()))?
            .insert(id.clone(), session);

        self.spawn_reader_thread(app.clone(), id.clone(), reader);

        Ok(id)
    }

    /// Reads PTY output on a dedicated OS thread for the lifetime of the
    /// session, forwarding each chunk as a `terminal:output` event, and
    /// emits one `terminal:exit` event once the pty closes (child exited or
    /// was killed).
    fn spawn_reader_thread(&self, app: AppHandle, terminal_id: String, mut reader: Box<dyn Read + Send>) {
        let sessions = Arc::clone(&self.sessions);
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let payload = TerminalOutputPayload {
                            terminal_id: terminal_id.clone(),
                            chunk: BASE64.encode(&buf[..n]),
                        };
                        if events::emit(&app, events::TERMINAL_OUTPUT, payload).is_err() {
                            break;
                        }
                    }
                    // A closed/killed pty typically surfaces as a read
                    // error on Windows rather than a clean `Ok(0)` EOF.
                    Err(_) => break,
                }
            }

            let exit_code = sessions
                .lock()
                .ok()
                .and_then(|mut sessions| sessions.get_mut(&terminal_id).and_then(|s| s.child.wait().ok()))
                .map(|status| status.exit_code() as i32);

            let _ = events::emit(
                &app,
                events::TERMINAL_EXIT,
                TerminalExitPayload { terminal_id, exit_code },
            );
        });
    }

    /// Writes raw bytes (from the frontend's xterm.js `onData`) to the
    /// session's pty stdin.
    pub fn write(&self, terminal_id: &str, data: &[u8]) -> AppResult<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::Other("terminal registry lock poisoned".to_string()))?;
        let session = sessions
            .get_mut(terminal_id)
            .ok_or_else(|| AppError::NotFound(format!("terminal session '{terminal_id}' not found")))?;
        session
            .writer
            .write_all(data)
            .map_err(|e| AppError::Other(format!("failed to write to terminal: {e}")))?;
        session
            .writer
            .flush()
            .map_err(|e| AppError::Other(format!("failed to flush terminal input: {e}")))
    }

    /// Resizes the pty to match the frontend terminal's new dimensions.
    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> AppResult<()> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::Other("terminal registry lock poisoned".to_string()))?;
        let session = sessions
            .get(terminal_id)
            .ok_or_else(|| AppError::NotFound(format!("terminal session '{terminal_id}' not found")))?;
        session
            .master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| AppError::Other(format!("failed to resize terminal: {e}")))
    }

    /// Kills the session's child process and drops its pty handles. The
    /// background reader thread notices the pty closing on its own and
    /// emits the final `terminal:exit` event.
    pub fn kill(&self, terminal_id: &str) -> AppResult<()> {
        let mut session = self
            .sessions
            .lock()
            .map_err(|_| AppError::Other("terminal registry lock poisoned".to_string()))?
            .remove(terminal_id)
            .ok_or_else(|| AppError::NotFound(format!("terminal session '{terminal_id}' not found")))?;

        session
            .child
            .kill()
            .map_err(|e| AppError::Other(format!("failed to kill terminal process: {e}")))?;
        // Reap the process so it doesn't linger as a zombie; ignore the
        // result, `kill` already succeeded and that's what the caller cares
        // about.
        let _ = session.child.wait();
        Ok(())
    }
}

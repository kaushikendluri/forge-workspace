//! Platform differences the terminal and git layers need to paper over
//! (shell selection, PATH resolution, env inheritance) live behind this
//! trait so `terminal/mod.rs` and `git/mod.rs` don't branch on `cfg!(target_os)`
//! themselves.

use std::collections::HashMap;
use std::path::PathBuf;

pub mod macos;
pub mod windows;

/// Per-OS behavior needed by the terminal (M3) and git (M2+) layers. `Send +
/// Sync` so a `Box<dyn OperatingSystemAdapter>` can live in `AppState`,
/// which Tauri requires to be `Send + Sync + 'static` to `.manage()` it.
pub trait OperatingSystemAdapter: Send + Sync {
    /// The shell to launch when the user opens a Terminal tab with no
    /// project-specific override (e.g. `powershell.exe` on Windows, `zsh`
    /// on macOS).
    fn default_shell(&self) -> String;

    /// Full argv used to invoke the default shell as an interactive login
    /// shell, e.g. `["zsh", "-il"]` on macOS or `["powershell.exe", "-NoLogo"]`
    /// on Windows.
    fn shell_invocation(&self) -> Vec<String>;

    /// Full argv to run `command` as a single non-interactive shell command
    /// and exit, e.g. `["zsh", "-lc", command]` on macOS or
    /// `["powershell.exe", "-NoLogo", "-NonInteractive", "-Command",
    /// command]` on Windows — used by the agent's `run_command` tool (M6),
    /// which needs a one-shot invocation with a captured exit code rather
    /// than the interactive session `shell_invocation()` hands the Terminal
    /// tab's PTY.
    fn one_shot_shell_invocation(&self, command: &str) -> Vec<String>;

    /// Resolves `name` (e.g. "git") to an absolute executable path using the
    /// platform's PATH-like search rules.
    fn resolve_executable(&self, name: &str) -> Option<PathBuf>;

    /// Environment variables a spawned shell/process should inherit,
    /// merged with the current process's own environment.
    fn env_vars(&self) -> HashMap<String, String>;

    /// The current user's home directory.
    fn home_dir(&self) -> Option<PathBuf>;
}

/// Returns the `OperatingSystemAdapter` for the platform this binary was
/// compiled for.
pub fn current() -> Box<dyn OperatingSystemAdapter> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsAdapter)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsAdapter)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        compile_error!("forge-workspace only targets Windows and macOS in Phase 1");
    }
}

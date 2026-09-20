//! Shared child-process execution: spawn a one-shot shell command with a
//! timeout and cancellation, used by both `agent::tools`'s `run_command`/
//! `run_tests`/`run_linter`/`run_build` tools and (M12)
//! `commands::testing_commands::run_test_suite`'s manual test/lint/build
//! runs — one implementation of the spawn/timeout/cancel dance, not two.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::os_adapter::OperatingSystemAdapter;

/// What running one shell command produced.
pub enum ProcessOutcome {
    /// The process ran to completion (however it exited).
    Finished { stdout: String, stderr: String, exit_code: Option<i32>, success: bool },
    /// `timeout` elapsed before the process finished; it was killed.
    TimedOut { timeout_ms: u128 },
    /// `cancel` fired before the process finished; it was killed.
    Cancelled,
    /// The process could not even be spawned.
    SpawnFailed(String),
}

/// Runs `command` as a single non-interactive shell invocation (via
/// `os_adapter.one_shot_shell_invocation`) inside `cwd`, killing it if
/// either `timeout` elapses or `cancel` fires first.
/// `kill_on_drop(true)` (set on the child) is what actually kills the OS
/// process when `tokio::select!` drops the losing branch's future — not the
/// `select!` itself.
pub async fn run_shell_command(
    os_adapter: &dyn OperatingSystemAdapter,
    cwd: &Path,
    command: &str,
    timeout: Duration,
    cancel: CancellationToken,
) -> ProcessOutcome {
    let argv = os_adapter.one_shot_shell_invocation(command);
    let Some((program, args)) = argv.split_first() else {
        return ProcessOutcome::SpawnFailed("no shell configured for this platform".to_string());
    };

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    cmd.current_dir(cwd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ProcessOutcome::SpawnFailed(format!("failed to run command: {e}")),
    };

    let output_fut = child.wait_with_output();
    tokio::select! {
        biased;
        _ = cancel.cancelled() => ProcessOutcome::Cancelled,
        _ = tokio::time::sleep(timeout) => ProcessOutcome::TimedOut { timeout_ms: timeout.as_millis() },
        result = output_fut => match result {
            Ok(output) => ProcessOutcome::Finished {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code(),
                success: output.status.success(),
            },
            Err(e) => ProcessOutcome::SpawnFailed(format!("failed to run command: {e}")),
        },
    }
}

//! M16: browser automation for the agent's frontend-verification tools
//! (`browser_open`/`browser_click`/`browser_type`/`browser_screenshot` in
//! `agent::tools`) plus the visual-regression snapshots they produce
//! (`db::repository::visual_snapshots`, `commands::visual_commands`).
//!
//! Real browser control goes through `chromiumoxide` (a pure-Rust Chrome
//! DevTools Protocol client — no bundled Node/Playwright runtime, drives
//! whatever real Chrome/Chromium/Edge binary is actually installed) behind
//! the [`BrowserBackend`] trait, mirroring `GitService`/
//! `OperatingSystemAdapter`'s trait-based testability precisely, and for the
//! same reason: CI has no guaranteed Chrome/Chromium/Edge binary or display
//! to actually launch against, so every test in this crate runs against
//! [`FakeBrowserBackend`] instead — the real [`chromium::ChromiumoxideBrowserBackend`]
//! is exercised only by hand, never by `cargo test`. See that module's own
//! docs, and this milestone's final report, for exactly what that leaves
//! unverified.
//!
//! [`BrowserManager`] is the per-run session registry — parallel to
//! `terminal::TerminalManager` (M3), but keyed by `agent_runs.id` rather
//! than a frontend-issued id: the agent tools never see a raw session id at
//! all, `BrowserManager` resolves "this run's session" itself, lazily
//! launching one (headless) on the first `browser_open` call and reusing it
//! for every later browser tool call in the same run.
//! `agent::tool_loop::finish_run` calls [`BrowserManager::close_for_run`] on
//! every terminal run-completion path (completed/failed/stopped, including
//! the top-level unexpected-error path), so a run that used the browser
//! never leaves a real Chrome process behind — see that function's own docs.

pub mod chromium;

pub use chromium::ChromiumoxideBrowserBackend;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::error::{AppError, AppResult};

/// Opaque id for one live browser session — never surfaced to the model or
/// the frontend, only used internally between [`BrowserManager`] and a
/// [`BrowserBackend`] implementation.
pub type BrowserSessionId = String;

/// Real browser control, behind a trait for the same reason `GitService`/
/// `OperatingSystemAdapter` are traits: so a fake implementation
/// ([`FakeBrowserBackend`]) can stand in for tests that would otherwise
/// require a real, installed Chrome/Chromium/Edge binary and a display.
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    /// Launches a new session (headless unless `headless` is false) and
    /// returns its id. Must fail with a clear, honest error — never
    /// silently no-op or fabricate a session — when no Chrome/Chromium/Edge
    /// binary can be found on this machine.
    async fn launch(&self, headless: bool) -> AppResult<BrowserSessionId>;
    /// Navigates `session_id`'s current page to `url`.
    async fn navigate(&self, session_id: &str, url: &str) -> AppResult<()>;
    /// Clicks the first element matching `selector`.
    async fn click(&self, session_id: &str, selector: &str) -> AppResult<()>;
    /// Types `text` into the first element matching `selector`.
    async fn type_text(&self, session_id: &str, selector: &str, text: &str) -> AppResult<()>;
    /// Captures a real, full-page PNG screenshot of the session's current
    /// page — never fabricated bytes.
    async fn screenshot(&self, session_id: &str) -> AppResult<Vec<u8>>;
    /// Closes the session and releases whatever real OS resources (browser
    /// process, CDP connection) it holds. A no-op is acceptable if
    /// `session_id` is already unknown to this backend — callers
    /// ([`BrowserManager`]) only ever call this for a session id they
    /// themselves handed out.
    async fn close(&self, session_id: &str) -> AppResult<()>;
}

/// Per-agent-run browser session registry, held in `AppState`. See this
/// module's own docs for the full "one lazily-launched session per run,
/// closed on every terminal path" story.
pub struct BrowserManager {
    backend: Arc<dyn BrowserBackend>,
    sessions_by_run: Mutex<HashMap<String, BrowserSessionId>>,
}

impl BrowserManager {
    pub fn new(backend: Arc<dyn BrowserBackend>) -> Self {
        Self { backend, sessions_by_run: Mutex::new(HashMap::new()) }
    }

    fn lock_sessions(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, BrowserSessionId>>> {
        self.sessions_by_run.lock().map_err(|_| AppError::Other("browser session registry lock poisoned".to_string()))
    }

    /// Returns `agent_run_id`'s session, launching a new headless one if
    /// this is its first browser tool call. Tool calls within one run are
    /// always dispatched sequentially (`agent::tool_loop` awaits each one
    /// before starting the next), so there is no concurrent-launch race to
    /// guard against here in practice.
    async fn session_for_run(&self, agent_run_id: &str) -> AppResult<BrowserSessionId> {
        let existing = { self.lock_sessions()?.get(agent_run_id).cloned() };
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = self.backend.launch(true).await?;
        self.lock_sessions()?.insert(agent_run_id.to_string(), id.clone());
        Ok(id)
    }

    pub async fn navigate(&self, agent_run_id: &str, url: &str) -> AppResult<()> {
        let session_id = self.session_for_run(agent_run_id).await?;
        self.backend.navigate(&session_id, url).await
    }

    pub async fn click(&self, agent_run_id: &str, selector: &str) -> AppResult<()> {
        let session_id = self.session_for_run(agent_run_id).await?;
        self.backend.click(&session_id, selector).await
    }

    pub async fn type_text(&self, agent_run_id: &str, selector: &str, text: &str) -> AppResult<()> {
        let session_id = self.session_for_run(agent_run_id).await?;
        self.backend.type_text(&session_id, selector, text).await
    }

    /// Captures a real screenshot of `agent_run_id`'s current page. Callers
    /// (`agent::tools::browser_screenshot_tool`) are responsible for saving
    /// the returned bytes to disk and recording a `visual_snapshots` row —
    /// this only ever returns real, freshly-captured bytes, never anything
    /// fabricated or cached.
    pub async fn screenshot(&self, agent_run_id: &str) -> AppResult<Vec<u8>> {
        let session_id = self.session_for_run(agent_run_id).await?;
        self.backend.screenshot(&session_id).await
    }

    /// Closes and forgets `agent_run_id`'s session, if it ever launched
    /// one — a no-op (not an error) for the common case of a run that never
    /// called a browser tool at all. Called from `agent::tool_loop::finish_run`
    /// so this runs on every terminal run-completion path.
    pub async fn close_for_run(&self, agent_run_id: &str) -> AppResult<()> {
        let session_id = { self.lock_sessions()?.remove(agent_run_id) };
        match session_id {
            Some(id) => self.backend.close(&id).await,
            None => Ok(()),
        }
    }
}

/// A fake [`BrowserBackend`] that never touches a real browser — every call
/// is recorded (so a test can assert on it) and answers with a small,
/// deterministic result. Used by this module's own tests, `agent::tools`'s
/// dispatch tests, and anywhere else in the crate that needs a
/// `BrowserManager` without a real Chrome instance. Visible crate-wide (not
/// `pub(crate)`) so it's usable from `agent::tools`'s test module, but only
/// compiled in under `cfg(test)` — it is never part of a release build.
#[cfg(test)]
#[derive(Default)]
pub struct FakeBrowserBackend {
    /// When true, `launch` fails the same honest way a real "no Chrome
    /// found" failure would — exercises that path without needing an
    /// actual missing/present browser binary.
    pub fail_launch: bool,
    pub screenshot_bytes: Vec<u8>,
    /// `pub` (like every other field here) so struct-update syntax
    /// (`FakeBrowserBackend { fail_launch: true, ..Default::default() }`)
    /// works from other modules' tests too — Rust's functional record
    /// update needs read access to every field of the base value, not just
    /// the ones actually named.
    pub launch_count: Mutex<u32>,
    pub navigate_calls: Mutex<Vec<(String, String)>>,
    pub click_calls: Mutex<Vec<(String, String)>>,
    pub type_calls: Mutex<Vec<(String, String, String)>>,
    pub screenshot_calls: Mutex<Vec<String>>,
    pub close_calls: Mutex<Vec<String>>,
}

#[cfg(test)]
#[async_trait]
impl BrowserBackend for FakeBrowserBackend {
    async fn launch(&self, _headless: bool) -> AppResult<BrowserSessionId> {
        if self.fail_launch {
            return Err(AppError::Other(
                "no Chrome, Chromium, or Edge installation could be found on this machine (fake)".to_string(),
            ));
        }
        let mut count = self.launch_count.lock().unwrap();
        *count += 1;
        Ok(format!("fake-session-{count}"))
    }

    async fn navigate(&self, session_id: &str, url: &str) -> AppResult<()> {
        self.navigate_calls.lock().unwrap().push((session_id.to_string(), url.to_string()));
        Ok(())
    }

    async fn click(&self, session_id: &str, selector: &str) -> AppResult<()> {
        self.click_calls.lock().unwrap().push((session_id.to_string(), selector.to_string()));
        Ok(())
    }

    async fn type_text(&self, session_id: &str, selector: &str, text: &str) -> AppResult<()> {
        self.type_calls.lock().unwrap().push((session_id.to_string(), selector.to_string(), text.to_string()));
        Ok(())
    }

    async fn screenshot(&self, session_id: &str) -> AppResult<Vec<u8>> {
        self.screenshot_calls.lock().unwrap().push(session_id.to_string());
        Ok(self.screenshot_bytes.clone())
    }

    async fn close(&self, session_id: &str) -> AppResult<()> {
        self.close_calls.lock().unwrap().push(session_id.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_for_run_launches_once_and_reuses_the_session_on_later_calls() {
        let backend = Arc::new(FakeBrowserBackend::default());
        let manager = BrowserManager::new(backend.clone());

        manager.navigate("run1", "https://example.com").await.expect("navigate");
        manager.click("run1", "button").await.expect("click");

        assert_eq!(*backend.launch_count.lock().unwrap(), 1, "a second browser tool call in the same run must reuse the session");
        let navigate_calls = backend.navigate_calls.lock().unwrap();
        let click_calls = backend.click_calls.lock().unwrap();
        assert_eq!(navigate_calls[0].0, click_calls[0].0, "both calls must have gone to the same session id");
    }

    #[tokio::test]
    async fn different_runs_get_isolated_sessions() {
        let backend = Arc::new(FakeBrowserBackend::default());
        let manager = BrowserManager::new(backend.clone());

        manager.navigate("run1", "https://a.example").await.expect("navigate run1");
        manager.navigate("run2", "https://b.example").await.expect("navigate run2");

        assert_eq!(*backend.launch_count.lock().unwrap(), 2, "two different runs must get two different sessions");
        let navigate_calls = backend.navigate_calls.lock().unwrap();
        assert_ne!(navigate_calls[0].0, navigate_calls[1].0);
    }

    #[tokio::test]
    async fn close_for_run_with_no_session_is_a_harmless_no_op() {
        let backend = Arc::new(FakeBrowserBackend::default());
        let manager = BrowserManager::new(backend.clone());

        manager.close_for_run("never-touched-the-browser").await.expect("close_for_run must not error");
        assert!(backend.close_calls.lock().unwrap().is_empty(), "the backend must never be asked to close a session that never existed");
    }

    #[tokio::test]
    async fn close_for_run_closes_and_forgets_the_session_exactly_once() {
        let backend = Arc::new(FakeBrowserBackend::default());
        let manager = BrowserManager::new(backend.clone());
        manager.navigate("run1", "https://example.com").await.expect("navigate");

        manager.close_for_run("run1").await.expect("close_for_run");
        assert_eq!(backend.close_calls.lock().unwrap().len(), 1);

        // Calling it again (mirroring `finish_run` potentially being
        // reached more than once in principle) must stay a no-op — the
        // session was already removed from the registry.
        manager.close_for_run("run1").await.expect("close_for_run again");
        assert_eq!(backend.close_calls.lock().unwrap().len(), 1, "closing an already-closed run must not call the backend again");
    }

    #[tokio::test]
    async fn a_session_launched_after_close_for_run_is_a_genuinely_new_one() {
        let backend = Arc::new(FakeBrowserBackend::default());
        let manager = BrowserManager::new(backend.clone());
        manager.navigate("run1", "https://example.com").await.expect("navigate");
        manager.close_for_run("run1").await.expect("close_for_run");

        manager.navigate("run1", "https://example.com/again").await.expect("navigate again");
        assert_eq!(*backend.launch_count.lock().unwrap(), 2, "a new browser tool call after close must launch a fresh session");
    }

    #[tokio::test]
    async fn a_launch_failure_surfaces_honestly_from_every_browser_manager_method() {
        let backend = Arc::new(FakeBrowserBackend { fail_launch: true, ..Default::default() });
        let manager = BrowserManager::new(backend);

        let err = manager.navigate("run1", "https://example.com").await.expect_err("must fail, never silently succeed");
        assert!(err.to_string().contains("Chrome"), "the error must be the honest 'no browser found' message: {err}");

        let err = manager.screenshot("run1").await.expect_err("screenshot must fail the same way");
        assert!(err.to_string().contains("Chrome"));
    }
}

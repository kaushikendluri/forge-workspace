//! M16: the real [`BrowserBackend`] implementation, driving an actual local
//! Chrome/Chromium/Edge binary over the Chrome DevTools Protocol via the
//! `chromiumoxide` crate.
//!
//! **Never exercised by `cargo test`/CI.** CI runners have no guaranteed
//! Chrome/Chromium/Edge binary or display to launch a real browser against,
//! so nothing in this file's actual launch/navigate/click/screenshot logic
//! runs during `cargo test` — see `browser::mod`'s own docs, and this
//! milestone's final report, for exactly what that leaves unverified.
//! `cargo check`/`cargo test` passing proves this compiles correctly against
//! chromiumoxide's real API (correct types, correct method names); it does
//! **not** prove a real browser actually launches end-to-end. That can only
//! be verified by hand, on a machine with a real Chrome/Chromium/Edge
//! installation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures_util::StreamExt;

use crate::error::{AppError, AppResult};
use crate::os_adapter::OperatingSystemAdapter;

use super::{BrowserBackend, BrowserSessionId};

/// Common install locations for a Chrome/Chromium/Edge binary that isn't
/// necessarily on `PATH`, per platform (this crate only targets Windows and
/// macOS — see `os_adapter::current`). Consulted only *after*
/// [`OperatingSystemAdapter::resolve_executable`]'s own `PATH` search comes
/// up empty, mirroring how that same adapter already resolves `git`.
#[cfg(target_os = "windows")]
const CANDIDATE_PATHS: &[&str] = &[
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Chromium\Application\chrome.exe",
];

#[cfg(target_os = "macos")]
const CANDIDATE_PATHS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const CANDIDATE_PATHS: &[&str] = &[];

/// Finds a real, usable Chrome/Chromium/Edge executable: first via the OS
/// adapter's own `PATH` search under a handful of common binary names, then
/// falling back to [`CANDIDATE_PATHS`]'s common install locations. `None`
/// means honestly none was found — callers must surface a clear error, never
/// silently proceed or fabricate a session.
pub fn find_chrome_executable(os_adapter: &dyn OperatingSystemAdapter) -> Option<PathBuf> {
    for name in ["google-chrome", "chromium", "chromium-browser", "chrome", "msedge"] {
        if let Some(path) = os_adapter.resolve_executable(name) {
            return Some(path);
        }
    }
    CANDIDATE_PATHS.iter().map(|p| PathBuf::from(*p)).find(|p| p.is_file())
}

struct LiveSession {
    browser: Browser,
    page: Page,
}

/// The real `BrowserBackend`. One `Browser` (real OS process) + `Page` per
/// session, keyed by the session id `BrowserManager` hands out; each
/// session's own CDP event-handler loop is spawned detached (fire-and-forget,
/// matching how this crate already spawns other background tasks via
/// `tauri::async_runtime::spawn`) and simply ends on its own once that
/// session's `Browser` is closed/dropped.
pub struct ChromiumoxideBrowserBackend {
    os_adapter: Box<dyn OperatingSystemAdapter>,
    sessions: Mutex<HashMap<BrowserSessionId, LiveSession>>,
}

impl ChromiumoxideBrowserBackend {
    pub fn new(os_adapter: Box<dyn OperatingSystemAdapter>) -> Self {
        Self { os_adapter, sessions: Mutex::new(HashMap::new()) }
    }

    fn lock_sessions(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<BrowserSessionId, LiveSession>>> {
        self.sessions.lock().map_err(|_| AppError::Other("browser session registry lock poisoned".to_string()))
    }

    /// A cheap clone of the session's `Page` handle (chromiumoxide's `Page`
    /// wraps an `Arc` internally) — cloned out from behind the lock so the
    /// actual CDP call can `.await` without holding a `std::sync::Mutex`
    /// guard across that await point.
    fn page_for(&self, session_id: &str) -> AppResult<Page> {
        let sessions = self.lock_sessions()?;
        sessions
            .get(session_id)
            .map(|s| s.page.clone())
            .ok_or_else(|| AppError::NotFound(format!("browser session '{session_id}' not found")))
    }
}

#[async_trait]
impl BrowserBackend for ChromiumoxideBrowserBackend {
    async fn launch(&self, headless: bool) -> AppResult<BrowserSessionId> {
        let executable = find_chrome_executable(self.os_adapter.as_ref()).ok_or_else(|| {
            AppError::Other(
                "no Chrome, Chromium, or Edge installation could be found on this machine — install one of \
                 these browsers to use the browser automation tools."
                    .to_string(),
            )
        })?;

        let mut builder = BrowserConfig::builder().chrome_executable(&executable);
        builder = if headless { builder.new_headless_mode() } else { builder.with_head() };
        let config = builder.build().map_err(|e| AppError::Other(format!("failed to configure browser launch: {e}")))?;

        // `mut`: some of these handles' methods (e.g. closing the browser
        // later — see `close` below) need `&mut self`; declared here rather
        // than only where strictly required so this compiles regardless of
        // exactly which chromiumoxide methods do.
        let (mut browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| AppError::Other(format!("failed to launch {}: {e}", executable.display())))?;

        // The CDP event/response loop — required for the `Browser`/`Page`
        // handles above to actually work at all. Spawned detached; it ends
        // on its own once this session's `Browser` is closed (see `close`
        // below), so there is nothing further to join here.
        tauri::async_runtime::spawn(async move {
            while handler.next().await.is_some() {}
        });

        let page = browser.new_page("about:blank").await.map_err(|e| AppError::Other(format!("failed to open a new page: {e}")))?;

        let id = uuid::Uuid::new_v4().to_string();
        self.lock_sessions()?.insert(id.clone(), LiveSession { browser, page });
        Ok(id)
    }

    async fn navigate(&self, session_id: &str, url: &str) -> AppResult<()> {
        let page = self.page_for(session_id)?;
        page.goto(url).await.map_err(|e| AppError::Other(format!("failed to navigate to '{url}': {e}")))?;
        Ok(())
    }

    async fn click(&self, session_id: &str, selector: &str) -> AppResult<()> {
        let page = self.page_for(session_id)?;
        let element =
            page.find_element(selector).await.map_err(|e| AppError::Other(format!("no element matching '{selector}' was found: {e}")))?;
        element.click().await.map_err(|e| AppError::Other(format!("failed to click '{selector}': {e}")))?;
        Ok(())
    }

    async fn type_text(&self, session_id: &str, selector: &str, text: &str) -> AppResult<()> {
        let page = self.page_for(session_id)?;
        let element =
            page.find_element(selector).await.map_err(|e| AppError::Other(format!("no element matching '{selector}' was found: {e}")))?;
        element.type_str(text).await.map_err(|e| AppError::Other(format!("failed to type into '{selector}': {e}")))?;
        Ok(())
    }

    async fn screenshot(&self, session_id: &str) -> AppResult<Vec<u8>> {
        let page = self.page_for(session_id)?;
        let params = ScreenshotParams::builder().format(CaptureScreenshotFormat::Png).full_page(true).build();
        page.screenshot(params).await.map_err(|e| AppError::Other(format!("failed to capture screenshot: {e}")))
    }

    async fn close(&self, session_id: &str) -> AppResult<()> {
        let session = { self.lock_sessions()?.remove(session_id) };
        if let Some(mut session) = session {
            // Best-effort: a real browser process that's already gone (e.g.
            // the user closed it, or it crashed) must not block the run from
            // reaching its terminal state over a close failure.
            let _ = session.browser.close().await;
            let _ = session.browser.wait().await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An OS adapter whose `resolve_executable` never finds anything — used
    /// to exercise `find_chrome_executable`'s "nothing on PATH" branch
    /// without depending on this machine's real PATH contents.
    struct NothingOnPathAdapter;
    impl OperatingSystemAdapter for NothingOnPathAdapter {
        fn default_shell(&self) -> String {
            "shell".to_string()
        }
        fn shell_invocation(&self) -> Vec<String> {
            vec![]
        }
        fn one_shot_shell_invocation(&self, _command: &str) -> Vec<String> {
            vec![]
        }
        fn resolve_executable(&self, _name: &str) -> Option<PathBuf> {
            None
        }
        fn env_vars(&self) -> HashMap<String, String> {
            HashMap::new()
        }
        fn home_dir(&self) -> Option<PathBuf> {
            None
        }
    }

    /// Pure logic test: when neither `PATH` nor any of the hardcoded
    /// install-location candidates exist on this machine, the lookup must
    /// come back `None` (never fabricate a path) — this is the only part of
    /// this file that's meaningfully testable without a real browser/display.
    /// On a CI runner that genuinely has no candidate path present, this
    /// also incidentally exercises the exact "nothing found" case
    /// `ChromiumoxideBrowserBackend::launch` turns into its honest error.
    #[test]
    fn find_chrome_executable_never_fabricates_a_path_that_does_not_exist() {
        let found = find_chrome_executable(&NothingOnPathAdapter);
        if let Some(path) = found {
            assert!(path.is_file(), "a returned path must genuinely exist on disk: {path:?}");
        }
        // `None` is also an entirely valid, honest outcome on a machine with
        // no browser installed at all — this test only asserts "never a
        // fabricated/nonexistent path", not "always finds one".
    }
}

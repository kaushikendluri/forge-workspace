//! Phase 5 M20: the `ModelProvider` trait — the seam that lets every real
//! AI-calling call site in this app (M6's `agent::tool_loop`, M8's
//! `orchestrator::planner`, M14's `agent::reviewer`, M15's
//! `agent::conflict_resolver`, M17's `brain`) run against whichever provider
//! a `model_configs` row actually names, instead of being hardwired to
//! `AnthropicClient` the way every one of those call sites was before this
//! milestone.
//!
//! This mirrors the `GitService`/`OperatingSystemAdapter`/`BrowserBackend`
//! trait-behind-`dyn` pattern already established elsewhere in this crate,
//! for the same reason: one seam, multiple real implementations, swappable
//! at a single factory ([`for_name`]) rather than scattered `match`es.
//!
//! The trait's two methods mirror `AnthropicClient::stream_turn`/
//! `request_structured_tool_call` exactly (see that module's own docs) —
//! this is a refactor of that call shape into a trait, not a redesign: the
//! shared [`crate::agent::anthropic_client::AssistantTurn`]/[`MessageParam`]/
//! [`ToolDefinition`]/[`StreamOutcome`] types defined there are reused
//! as-is, never reinvented per-provider. Every non-Anthropic provider is
//! responsible for translating those shared, Anthropic-shaped types into its
//! own wire format on the way out and translating its own response back into
//! them on the way in — never leaking a provider-specific response shape
//! past this trait. See each submodule's own docs for its translation and
//! exactly what's unverified without a live key for that provider.
//!
//! [`AnthropicProvider`] (`anthropic.rs`) wraps the existing, untouched
//! `AnthropicClient` — every one of that module's own unit tests keeps
//! passing unchanged, since this milestone never edits `anthropic_client.rs`
//! itself, only adds a thin trait `impl` beside it.

pub mod anthropic;
pub mod google;
pub mod openai;
pub mod openrouter;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

pub use crate::agent::anthropic_client::{MessageParam, StreamOutcome, ToolDefinition};
use crate::error::{AppError, AppResult};
use crate::secrets;

/// One real AI provider, behind a trait so every call site above can be
/// written once against `&dyn ModelProvider` and run against whichever
/// provider a resolved `model_configs` row names. `Send + Sync` so a boxed
/// instance can be held across the `.await` points every call site already
/// has (matching `BrowserBackend`'s own bound).
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Streams one assistant turn, calling `on_text_delta` for every
    /// streamed text fragment as it arrives (a live "typing" UI — not
    /// persisted). Mirrors `AnthropicClient::stream_turn` exactly; see that
    /// method's own docs for the cancellation/retry contract every caller
    /// relies on. `on_text_delta` is `&mut dyn FnMut` (not `impl FnMut`,
    /// which `AnthropicClient`'s own inherent method still uses) purely
    /// because a trait method can't be generic and still be `dyn`-callable
    /// — callers pass `&mut closure` instead of the closure itself. The
    /// `for<'a>` is written explicitly (rather than relying on the usual
    /// elision to higher-rank it) because `#[async_trait]`'s signature
    /// rewriting otherwise collapses the elided lifetime to one concrete
    /// lifetime tied to the call, which then fails to unify with the plain
    /// (non-async, properly-elided) functions each real implementation
    /// forwards to — every `impl ModelProvider` and every free function it
    /// delegates to must repeat this exact `for<'a>` form, not just
    /// `dyn FnMut(&str)`.
    async fn stream_turn(
        &self,
        model: &str,
        max_tokens: u32,
        system: &str,
        messages: &[MessageParam],
        tools: &[ToolDefinition],
        cancel: &CancellationToken,
        on_text_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> AppResult<StreamOutcome>;

    /// Sends one non-streaming, forced-tool-choice request, guaranteeing the
    /// response is exactly one call to `tool.name`. Mirrors
    /// `AnthropicClient::request_structured_tool_call` exactly; used by M8's
    /// planner, M14's reviewer verdict, and M17's brain analysis.
    async fn request_structured_tool_call(
        &self,
        model: &str,
        max_tokens: u32,
        system: &str,
        messages: &[MessageParam],
        tool: &ToolDefinition,
        cancel: &CancellationToken,
    ) -> AppResult<StreamOutcome>;
}

/// The four provider names a `model_configs.provider` column is expected to
/// hold — the only strings [`secret_key_for`]/[`display_name`]/[`for_name`]
/// recognize. Anything else is a real, honest error (an unknown provider
/// string in the database, e.g. from a future migration this binary doesn't
/// know about yet) rather than a silent fallback to Anthropic.
const KNOWN_PROVIDERS: &[&str] = &["anthropic", "openai", "google", "openrouter"];

fn unknown_provider_err(provider_name: &str) -> AppError {
    AppError::InvalidInput(format!(
        "unknown model provider '{provider_name}' (expected one of {})",
        KNOWN_PROVIDERS.join(", ")
    ))
}

/// The OS-keychain account name (see `crate::secrets`) backing `provider_name`'s
/// API key. Every real call site fetches its key through this rather than a
/// provider-specific constant, so adding a fifth provider later only touches
/// this `match` (and the three sibling functions below), never the call
/// sites themselves.
pub fn secret_key_for(provider_name: &str) -> AppResult<&'static str> {
    match provider_name {
        "anthropic" => Ok(secrets::ANTHROPIC_API_KEY),
        "openai" => Ok(secrets::OPENAI_API_KEY),
        "google" => Ok(secrets::GOOGLE_API_KEY),
        "openrouter" => Ok(secrets::OPENROUTER_API_KEY),
        other => Err(unknown_provider_err(other)),
    }
}

/// A short, human-readable name for `provider_name`, for "No X API key is
/// configured" messages — every call site's existing wording is kept
/// verbatim, just parameterized by this instead of hardcoding "Anthropic".
pub fn display_name(provider_name: &str) -> &'static str {
    match provider_name {
        "anthropic" => "Anthropic",
        "openai" => "OpenAI",
        "google" => "Google",
        "openrouter" => "OpenRouter",
        _ => "the configured",
    }
}

/// Builds the real [`ModelProvider`] for `provider_name`, given an
/// already-fetched `api_key` (every call site fetches it first via
/// [`secret_key_for`] + `secrets::get_secret`, on the blocking pool, the same
/// pattern `AnthropicClient::new` callers already used — this function
/// itself does no I/O).
pub fn for_name(provider_name: &str, api_key: String) -> AppResult<Box<dyn ModelProvider>> {
    match provider_name {
        "anthropic" => Ok(Box::new(self::anthropic::AnthropicProvider::new(api_key)?)),
        "openai" => Ok(Box::new(self::openai::OpenAIProvider::new(api_key)?)),
        "google" => Ok(Box::new(self::google::GoogleProvider::new(api_key)?)),
        "openrouter" => Ok(Box::new(self::openrouter::OpenRouterProvider::new(api_key)?)),
        other => Err(unknown_provider_err(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_key_for_covers_every_known_provider_distinctly() {
        let keys: Vec<&str> = KNOWN_PROVIDERS.iter().map(|p| secret_key_for(p).expect("known provider")).collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "every provider must have its own distinct keychain account");
    }

    #[test]
    fn secret_key_for_rejects_an_unknown_provider() {
        let err = secret_key_for("made_up_provider").expect_err("should reject");
        assert!(err.to_string().contains("made_up_provider"));
    }

    #[test]
    fn for_name_rejects_an_unknown_provider_before_any_io() {
        // Not `.expect_err(...)`: the `Ok` type here is `Box<dyn
        // ModelProvider>`, which doesn't implement `Debug` (and shouldn't —
        // it's a live HTTP client, not a value worth debug-printing), and
        // `expect_err` requires `T: Debug` to format a panic message for the
        // (unreached) `Ok` case. Matching directly avoids that bound.
        let result = for_name("made_up_provider", "key".to_string());
        let Err(err) = result else { panic!("should reject an unknown provider") };
        assert!(err.to_string().contains("made_up_provider"));
    }

    #[test]
    fn display_name_is_never_empty_for_a_known_provider() {
        for provider in KNOWN_PROVIDERS {
            assert!(!display_name(provider).is_empty());
        }
    }
}

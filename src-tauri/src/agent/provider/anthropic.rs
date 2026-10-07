//! Phase 5 M20: `AnthropicProvider` — the existing, working M6
//! `AnthropicClient` wrapped behind [`super::ModelProvider`]. Deliberately a
//! thin wrapper rather than a move: `anthropic_client.rs` is untouched
//! (every one of its own unit tests keeps passing unchanged, unaffected by
//! this file existing at all), and this impl does nothing but delegate
//! straight through to that module's own, already-correct inherent methods.
//!
//! The only translation happening here is on `on_text_delta`: the trait
//! takes `&mut dyn FnMut(&str)` (so it's `dyn`-callable); `AnthropicClient`'s
//! inherent `stream_turn` takes `impl FnMut(&str)`. No conversion code is
//! actually needed for that, though — the standard library's blanket
//! `impl<F: FnMut<A>> FnMut<A> for &mut F` means a `&mut dyn FnMut(&str)`
//! already *is* an `FnMut(&str)`, so it satisfies the inherent method's
//! `impl FnMut(&str)` bound exactly as handed in.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::agent::anthropic_client::{AnthropicClient, MessageParam, StreamOutcome, ToolDefinition};
use crate::error::AppResult;

use super::ModelProvider;

/// Wraps a real [`AnthropicClient`]. `new` mirrors `AnthropicClient::new`
/// exactly (same error path for a malformed `reqwest::Client` build).
pub struct AnthropicProvider {
    inner: AnthropicClient,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> AppResult<Self> {
        Ok(Self { inner: AnthropicClient::new(api_key)? })
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    async fn stream_turn(
        &self,
        model: &str,
        max_tokens: u32,
        system: &str,
        messages: &[MessageParam],
        tools: &[ToolDefinition],
        cancel: &CancellationToken,
        on_text_delta: &mut (dyn FnMut(&str) + Send),
    ) -> AppResult<StreamOutcome> {
        self.inner.stream_turn(model, max_tokens, system, messages, tools, cancel, on_text_delta).await
    }

    async fn request_structured_tool_call(
        &self,
        model: &str,
        max_tokens: u32,
        system: &str,
        messages: &[MessageParam],
        tool: &ToolDefinition,
        cancel: &CancellationToken,
    ) -> AppResult<StreamOutcome> {
        self.inner.request_structured_tool_call(model, max_tokens, system, messages, tool, cancel).await
    }
}

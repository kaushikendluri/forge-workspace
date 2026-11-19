//! Phase 5 M20: `OpenRouterProvider` — OpenRouter's API is documented as
//! OpenAI-compatible (the same Chat Completions request/response/streaming
//! shape, just a different base URL and model-id namespacing like
//! `"anthropic/claude-sonnet-4.5"` or `"meta-llama/llama-3.1-70b-instruct"`).
//! Rather than reimplementing that shape a second time, this is a thin
//! wrapper delegating to `openai`'s own free `stream_turn_impl`/
//! `structured_call_impl` functions — the exact same request-building,
//! response-parsing, and streaming-accumulation code [`super::openai::OpenAIProvider`]
//! uses, just pointed at OpenRouter's endpoint. Authentication is the same
//! `Authorization: Bearer <key>` scheme too, so no header translation is
//! needed either.
//!
//! What's unverified without a live key: whether every OpenRouter-routed
//! upstream model actually honors `stream_options.include_usage` and the
//! forced `tool_choice`/streamed `tool_calls` shape identically to OpenAI
//! itself — OpenRouter's own docs say it normalizes to the OpenAI shape, but
//! a specific upstream model could still behave slightly differently.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::agent::anthropic_client::{MessageParam, StreamOutcome, ToolDefinition};
use crate::error::AppResult;

use super::openai::{structured_call_impl, stream_turn_impl};
use super::ModelProvider;

const API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

fn build_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| crate::error::AppError::Other(format!("failed to build HTTP client: {e}")))
}

pub struct OpenRouterProvider {
    http: reqwest::Client,
    api_key: String,
}

impl OpenRouterProvider {
    pub fn new(api_key: String) -> AppResult<Self> {
        Ok(Self { http: build_http_client()?, api_key })
    }
}

#[async_trait]
impl ModelProvider for OpenRouterProvider {
    async fn stream_turn(
        &self,
        model: &str,
        max_tokens: u32,
        system: &str,
        messages: &[MessageParam],
        tools: &[ToolDefinition],
        cancel: &CancellationToken,
        on_text_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> AppResult<StreamOutcome> {
        stream_turn_impl(&self.http, API_URL, &self.api_key, model, max_tokens, system, messages, tools, cancel, on_text_delta).await
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
        structured_call_impl(&self.http, API_URL, &self.api_key, model, max_tokens, system, messages, tool, cancel).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not much to unit-test here beyond "it constructs" — the real parsing
    /// logic is `openai`'s own, already covered by that module's fixture
    /// tests; this just confirms the wrapper points at a distinct URL/host
    /// rather than accidentally reusing OpenAI's.
    #[test]
    fn uses_openrouters_own_host_not_openais() {
        assert!(API_URL.contains("openrouter.ai"));
        assert!(!API_URL.contains("api.openai.com"));
    }

    #[test]
    fn constructs_successfully_with_a_plain_api_key() {
        assert!(OpenRouterProvider::new("sk-or-test".to_string()).is_ok());
    }
}

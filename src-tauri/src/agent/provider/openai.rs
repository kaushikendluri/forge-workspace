//! Phase 5 M20: `OpenAIProvider` — a real implementation against OpenAI's
//! documented Chat Completions API (`POST
//! https://api.openai.com/v1/chat/completions`), built from the documented
//! request/response/streaming shapes the same way `anthropic_client.rs` was
//! (M6) — no live API key to verify against in this environment, so getting
//! the wire format right from the spec and unit-testing the tricky part
//! (streamed `tool_calls` delta accumulation, split across `index`-keyed
//! chunks) against hand-built fixtures is the only verification available
//! here. See this milestone's final report for exactly what remains
//! unverified against a live key.
//!
//! ## Translating the shared (Anthropic-shaped) types
//!
//! [`MessageParam`]/[`ContentBlockParam`] (this crate's shared message shape,
//! reused as-is per `agent::provider`'s own docs) don't map 1:1 onto
//! OpenAI's `messages` array:
//!
//! - An assistant `MessageParam` can mix `Text` and `ToolUse` blocks in one
//!   turn; OpenAI represents that as one message with an optional `content`
//!   string *and* an optional `tool_calls` array side by side — so this
//!   collects every `Text` block's text into one `content` string and every
//!   `ToolUse` block into one `tool_calls` entry, both on the same message
//!   ([`to_openai_messages`]).
//! - Anthropic bundles multiple `ToolResult` blocks from one turn into a
//!   single `user`-role message's `content` array; OpenAI instead expects
//!   **one message per tool result**, each with `role: "tool"` and a
//!   `tool_call_id` — so one incoming "user" `MessageParam` full of
//!   `ToolResult` blocks explodes into N separate OpenAI messages.
//! - A plain `user_text` `MessageParam` (no tool results) becomes one
//!   ordinary `{"role": "user", "content": "..."}` message.
//!
//! The reverse direction (OpenAI's response back into
//! [`AssistantContentBlock`]/[`AssistantTurn`]) is the mirror image: a
//! `tool_calls` entry becomes a `ToolUse` block (with its `function.arguments`
//! JSON *string* parsed into a real `serde_json::Value`, the same
//! "parse once, after everything's accumulated" rule `anthropic_client`'s
//! `input_json_delta` handling already follows), and `content` becomes a
//! `Text` block.
//!
//! ## What's unverified without a live key
//!
//! - The exact streamed shape of `tool_calls` deltas (id/name arriving only
//!   on the first chunk for an index, `arguments` fragments on every chunk
//!   after) is built from OpenAI's documented streaming behavior, not
//!   observed against a live response.
//! - `stream_options: {"include_usage": true}` is sent so a final usage
//!   chunk arrives at all — unverified that every OpenAI-compatible
//!   deployment honors it identically.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::agent::anthropic_client::{
    AssistantContentBlock, AssistantTurn, ContentBlockParam, MessageParam, SseDecoder, StreamOutcome, ToolDefinition, TurnUsage,
};
use crate::error::{AppError, AppResult};

use super::ModelProvider;

const API_URL: &str = "https://api.openai.com/v1/chat/completions";

// ---------------------------------------------------------------------
// Shared-type -> OpenAI wire-format translation.
// ---------------------------------------------------------------------

/// Translates this crate's shared `system` + `messages` into OpenAI's
/// `messages` array (system message first), per this module's own docs.
fn to_openai_messages(system: &str, messages: &[MessageParam]) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": system })];

    for message in messages {
        if message.role == "assistant" {
            let mut text = String::new();
            let mut tool_calls = Vec::new();
            for block in &message.content {
                match block {
                    ContentBlockParam::Text { text: t } => text.push_str(t),
                    ContentBlockParam::ToolUse { id, name, input } => {
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": input.to_string() }
                        }));
                    }
                    // An assistant turn never carries a ToolResult in this
                    // crate's own shared shape (see `MessageParam::assistant`'s
                    // callers) — nothing to translate.
                    ContentBlockParam::ToolResult { .. } => {}
                }
            }
            let content = if text.is_empty() { Value::Null } else { Value::String(text) };
            let mut obj = serde_json::Map::new();
            obj.insert("role".to_string(), json!("assistant"));
            obj.insert("content".to_string(), content);
            if !tool_calls.is_empty() {
                obj.insert("tool_calls".to_string(), Value::Array(tool_calls));
            }
            out.push(Value::Object(obj));
            continue;
        }

        // A "user" `MessageParam`: either plain text, or one-or-more tool
        // results bundled together (see `MessageParam::user_tool_results`) —
        // each `ToolResult` block becomes its own `role: "tool"` message.
        for block in &message.content {
            match block {
                ContentBlockParam::Text { text } => out.push(json!({ "role": "user", "content": text })),
                ContentBlockParam::ToolResult { tool_use_id, content, is_error } => {
                    // OpenAI's `tool` message has no `is_error` field of its
                    // own; prefixing the content is the honest way to carry
                    // that signal through without inventing a field the API
                    // doesn't define.
                    let rendered = if *is_error { format!("Error: {content}") } else { content.clone() };
                    out.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": rendered }));
                }
                ContentBlockParam::ToolUse { .. } => {}
            }
        }
    }

    out
}

fn to_openai_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.input_schema } }))
        .collect()
}

// ---------------------------------------------------------------------
// Non-streaming response shape.
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ToolCallResponse {
    id: String,
    function: ToolCallFunction,
}

#[derive(Debug, Default, Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallResponse>>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    #[serde(default)]
    message: String,
}

fn parse_non_stream_response(bytes: &[u8]) -> AppResult<AssistantTurn> {
    let parsed: ChatCompletionResponse = serde_json::from_slice(bytes)
        .map_err(|e| AppError::Other(format!("failed to parse OpenAI response: {e} (raw: {})", String::from_utf8_lossy(bytes))))?;

    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Other("OpenAI response had no choices".to_string()))?;

    let mut content = Vec::new();
    if let Some(text) = choice.message.content.filter(|t| !t.is_empty()) {
        content.push(AssistantContentBlock::Text(text));
    }
    for tool_call in choice.message.tool_calls.unwrap_or_default() {
        let input = parse_tool_arguments(&tool_call.function.name, &tool_call.function.arguments)?;
        content.push(AssistantContentBlock::ToolUse { id: tool_call.id, name: tool_call.function.name, input });
    }

    Ok(AssistantTurn {
        content,
        stop_reason: choice.finish_reason,
        usage: TurnUsage { input_tokens: parsed.usage.prompt_tokens, output_tokens: parsed.usage.completion_tokens },
    })
}

fn parse_tool_arguments(name: &str, arguments: &str) -> AppResult<Value> {
    if arguments.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(arguments)
        .map_err(|e| AppError::Other(format!("tool call '{name}' produced invalid JSON arguments: {e} (accumulated: {arguments})")))
}

// ---------------------------------------------------------------------
// Streaming response shape + accumulator.
// ---------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct DeltaToolCallFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeltaToolCall {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaToolCallFunction>,
}

#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Default)]
struct InProgressToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Accumulates a sequence of `ChatCompletionChunk`s into one [`AssistantTurn`]
/// — the OpenAI-flavored equivalent of `anthropic_client`'s
/// `StreamAccumulator`. `tool_calls` is keyed by the chunk's own `index`
/// (OpenAI's streamed tool calls are "wide": several can be in flight at
/// once, each identified by position, not by a content-block index the way
/// Anthropic's are) — a `BTreeMap` so `finish()` emits them in a stable,
/// index order regardless of interleaving.
#[derive(Default)]
struct OpenAiStreamAccumulator {
    text: String,
    tool_calls: std::collections::BTreeMap<usize, InProgressToolCall>,
    finish_reason: Option<String>,
    usage: TurnUsage,
}

impl OpenAiStreamAccumulator {
    fn apply(&mut self, raw: &str, on_text_delta: &mut dyn FnMut(&str)) -> AppResult<()> {
        let chunk: ChatCompletionChunk =
            serde_json::from_str(raw).map_err(|e| AppError::Other(format!("failed to parse OpenAI stream chunk: {e} (raw: {raw})")))?;

        if let Some(usage) = chunk.usage {
            self.usage = TurnUsage { input_tokens: usage.prompt_tokens, output_tokens: usage.completion_tokens };
        }

        for choice in chunk.choices {
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(reason);
            }
            if let Some(text) = choice.delta.content {
                if !text.is_empty() {
                    on_text_delta(&text);
                    self.text.push_str(&text);
                }
            }
            for tool_call in choice.delta.tool_calls.into_iter().flatten() {
                let entry = self.tool_calls.entry(tool_call.index).or_default();
                if let Some(id) = tool_call.id {
                    entry.id = id;
                }
                if let Some(function) = tool_call.function {
                    if let Some(name) = function.name {
                        entry.name = name;
                    }
                    if let Some(arguments) = function.arguments {
                        entry.arguments.push_str(&arguments);
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> AppResult<AssistantTurn> {
        let mut content = Vec::new();
        if !self.text.is_empty() {
            content.push(AssistantContentBlock::Text(self.text));
        }
        for (_, tool_call) in self.tool_calls {
            let input = parse_tool_arguments(&tool_call.name, &tool_call.arguments)?;
            content.push(AssistantContentBlock::ToolUse { id: tool_call.id, name: tool_call.name, input });
        }
        Ok(AssistantTurn { content, stop_reason: self.finish_reason, usage: self.usage })
    }
}

// ---------------------------------------------------------------------
// HTTP layer — free functions so `OpenRouterProvider` (OpenAI-compatible,
// different base URL) can reuse them verbatim rather than reimplementing.
// ---------------------------------------------------------------------

fn build_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder().use_rustls_tls().build().map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))
}

async fn send_and_check(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: Value,
    cancel: &CancellationToken,
) -> AppResult<Option<reqwest::Response>> {
    let send_fut = http.post(url).bearer_auth(api_key).header("content-type", "application/json").json(&body).send();
    let response = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(None),
        result = send_fut => result.map_err(|e| AppError::Other(format!("OpenAI-compatible API request failed: {e}")))?,
    };
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<ApiErrorBody>(&text).map(|b| b.error.message).unwrap_or(text);
        return Err(AppError::Other(format!("OpenAI-compatible API returned HTTP {status}: {detail}")));
    }
    Ok(Some(response))
}

pub(super) async fn stream_turn_impl(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[MessageParam],
    tools: &[ToolDefinition],
    cancel: &CancellationToken,
    on_text_delta: &mut (dyn FnMut(&str) + Send),
) -> AppResult<StreamOutcome> {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": to_openai_messages(system, messages),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(to_openai_tools(tools));
    }

    let Some(response) = send_and_check(http, url, api_key, body, cancel).await? else {
        return Ok(StreamOutcome::Cancelled);
    };

    let mut stream = response.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut accumulator = OpenAiStreamAccumulator::default();

    loop {
        let next = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(StreamOutcome::Cancelled),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = next else { break };
        let bytes = chunk.map_err(|e| AppError::Other(format!("OpenAI-compatible API stream read failed: {e}")))?;
        for event in decoder.feed(&bytes) {
            if event.data.trim() == "[DONE]" {
                break;
            }
            accumulator.apply(&event.data, on_text_delta)?;
        }
    }

    Ok(StreamOutcome::Turn(accumulator.finish()?))
}

pub(super) async fn structured_call_impl(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    model: &str,
    max_tokens: u32,
    system: &str,
    messages: &[MessageParam],
    tool: &ToolDefinition,
    cancel: &CancellationToken,
) -> AppResult<StreamOutcome> {
    let body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": to_openai_messages(system, messages),
        "tools": to_openai_tools(std::slice::from_ref(tool)),
        "tool_choice": { "type": "function", "function": { "name": tool.name } },
        "stream": false,
    });

    let Some(response) = send_and_check(http, url, api_key, body, cancel).await? else {
        return Ok(StreamOutcome::Cancelled);
    };
    let body_fut = response.bytes();
    let bytes = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(StreamOutcome::Cancelled),
        result = body_fut => result.map_err(|e| AppError::Other(format!("OpenAI-compatible API response read failed: {e}")))?,
    };

    Ok(StreamOutcome::Turn(parse_non_stream_response(&bytes)?))
}

// ---------------------------------------------------------------------
// The provider itself.
// ---------------------------------------------------------------------

pub struct OpenAIProvider {
    http: reqwest::Client,
    api_key: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> AppResult<Self> {
        Ok(Self { http: build_http_client()?, api_key })
    }
}

#[async_trait]
impl ModelProvider for OpenAIProvider {
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

    fn sse_data(value: &Value) -> String {
        format!("data: {value}\n\n")
    }

    // -- to_openai_messages ------------------------------------------------

    #[test]
    fn translates_plain_user_text_and_system() {
        let messages = vec![MessageParam::user_text("hello")];
        let out = to_openai_messages("be nice", &messages);
        assert_eq!(out[0], json!({ "role": "system", "content": "be nice" }));
        assert_eq!(out[1], json!({ "role": "user", "content": "hello" }));
    }

    #[test]
    fn translates_an_assistant_turn_with_text_and_a_tool_use_into_one_message() {
        let messages = vec![MessageParam::assistant(vec![
            ContentBlockParam::Text { text: "Let me check.".to_string() },
            ContentBlockParam::ToolUse { id: "call_1".to_string(), name: "read_file".to_string(), input: json!({"path": "a.rs"}) },
        ])];
        let out = to_openai_messages("sys", &messages);
        let assistant_msg = &out[1];
        assert_eq!(assistant_msg["role"], "assistant");
        assert_eq!(assistant_msg["content"], "Let me check.");
        assert_eq!(assistant_msg["tool_calls"][0]["id"], "call_1");
        assert_eq!(assistant_msg["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(assistant_msg["tool_calls"][0]["function"]["arguments"], "{\"path\":\"a.rs\"}");
    }

    #[test]
    fn explodes_bundled_tool_results_into_separate_tool_messages() {
        let messages = vec![MessageParam::user_tool_results(vec![
            ContentBlockParam::ToolResult { tool_use_id: "call_1".to_string(), content: "ok".to_string(), is_error: false },
            ContentBlockParam::ToolResult { tool_use_id: "call_2".to_string(), content: "boom".to_string(), is_error: true },
        ])];
        let out = to_openai_messages("sys", &messages);
        assert_eq!(out.len(), 3, "system + two separate tool messages");
        assert_eq!(out[1], json!({ "role": "tool", "tool_call_id": "call_1", "content": "ok" }));
        assert_eq!(out[2], json!({ "role": "tool", "tool_call_id": "call_2", "content": "Error: boom" }));
    }

    #[test]
    fn an_assistant_turn_with_only_a_tool_use_has_null_content_not_an_empty_string() {
        let messages = vec![MessageParam::assistant(vec![ContentBlockParam::ToolUse {
            id: "call_1".to_string(),
            name: "git_status".to_string(),
            input: json!({}),
        }])];
        let out = to_openai_messages("sys", &messages);
        assert_eq!(out[1]["content"], Value::Null);
    }

    // -- parse_non_stream_response -----------------------------------------

    #[test]
    fn parses_a_non_stream_response_with_a_forced_tool_call() {
        let raw = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{ "id": "call_abc", "type": "function", "function": { "name": "propose_plan", "arguments": "{\"tasks\":[]}" } }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 20 }
        });
        let turn = parse_non_stream_response(raw.to_string().as_bytes()).expect("should parse");
        assert_eq!(turn.content.len(), 1);
        assert_eq!(
            turn.content[0],
            AssistantContentBlock::ToolUse { id: "call_abc".to_string(), name: "propose_plan".to_string(), input: json!({"tasks": []}) }
        );
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_calls"));
        assert_eq!(turn.usage, TurnUsage { input_tokens: 100, output_tokens: 20 });
    }

    #[test]
    fn parses_a_plain_text_response_with_no_tool_calls() {
        let raw = json!({
            "choices": [{ "message": { "content": "hello there" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 3 }
        });
        let turn = parse_non_stream_response(raw.to_string().as_bytes()).expect("should parse");
        assert_eq!(turn.content, vec![AssistantContentBlock::Text("hello there".to_string())]);
    }

    // -- streaming accumulator ----------------------------------------------

    /// The trickiest part of this provider's wire format: a tool call's
    /// `id`/`function.name` arrive only on the delta where that `index`
    /// first appears; every later delta for the same `index` carries only
    /// an `arguments` fragment to append — mirroring
    /// `anthropic_client`'s own `input_json_delta` accumulation test.
    #[test]
    fn accumulates_text_and_a_split_tool_call_across_chunks() {
        let mut raw = String::new();
        raw += &sse_data(&json!({ "choices": [{ "delta": { "content": "Checking" }, "finish_reason": null }] }));
        raw += &sse_data(&json!({ "choices": [{ "delta": { "content": " now." }, "finish_reason": null }] }));
        raw += &sse_data(&json!({
            "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "call_1", "function": { "name": "read_file", "arguments": "" } }] }, "finish_reason": null }]
        }));
        raw += &sse_data(&json!({
            "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "{\"pa" } }] }, "finish_reason": null }]
        }));
        raw += &sse_data(&json!({
            "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "th\":\"x.rs\"}" } }] }, "finish_reason": "tool_calls" }]
        }));
        raw += &sse_data(&json!({ "choices": [], "usage": { "prompt_tokens": 42, "completion_tokens": 9 } }));
        raw += "data: [DONE]\n\n";

        let mut decoder = SseDecoder::new();
        let events = decoder.feed(raw.as_bytes());
        let mut acc = OpenAiStreamAccumulator::default();
        let mut deltas = Vec::new();
        for event in &events {
            if event.data.trim() == "[DONE]" {
                continue;
            }
            acc.apply(&event.data, &mut |t: &str| deltas.push(t.to_string())).expect("apply should succeed");
        }
        let turn = acc.finish().expect("finish should succeed");

        assert_eq!(deltas, vec!["Checking".to_string(), " now.".to_string()]);
        assert_eq!(turn.content.len(), 2);
        assert_eq!(turn.content[0], AssistantContentBlock::Text("Checking now.".to_string()));
        assert_eq!(
            turn.content[1],
            AssistantContentBlock::ToolUse { id: "call_1".to_string(), name: "read_file".to_string(), input: json!({"path": "x.rs"}) }
        );
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_calls"));
        assert_eq!(turn.usage, TurnUsage { input_tokens: 42, output_tokens: 9 });
    }

    #[test]
    fn two_interleaved_tool_calls_accumulate_independently_by_index() {
        let mut raw = String::new();
        raw += &sse_data(&json!({ "choices": [{ "delta": { "tool_calls": [
            { "index": 0, "id": "call_a", "function": { "name": "fn_a", "arguments": "{\"x\":" } },
            { "index": 1, "id": "call_b", "function": { "name": "fn_b", "arguments": "{\"y\":" } }
        ] } }] }));
        raw += &sse_data(&json!({ "choices": [{ "delta": { "tool_calls": [
            { "index": 0, "function": { "arguments": "1}" } },
            { "index": 1, "function": { "arguments": "2}" } }
        ] }, "finish_reason": "tool_calls" }] }));

        let mut decoder = SseDecoder::new();
        let events = decoder.feed(raw.as_bytes());
        let mut acc = OpenAiStreamAccumulator::default();
        for event in &events {
            acc.apply(&event.data, &mut |_: &str| {}).expect("apply should succeed");
        }
        let turn = acc.finish().expect("finish should succeed");

        assert_eq!(turn.content.len(), 2);
        assert_eq!(
            turn.content[0],
            AssistantContentBlock::ToolUse { id: "call_a".to_string(), name: "fn_a".to_string(), input: json!({"x": 1}) }
        );
        assert_eq!(
            turn.content[1],
            AssistantContentBlock::ToolUse { id: "call_b".to_string(), name: "fn_b".to_string(), input: json!({"y": 2}) }
        );
    }

    #[test]
    fn malformed_accumulated_tool_arguments_fail_clearly_rather_than_panicking() {
        let mut acc = OpenAiStreamAccumulator::default();
        acc.tool_calls.insert(0, InProgressToolCall { id: "call_1".to_string(), name: "foo".to_string(), arguments: "{not json".to_string() });
        let err = acc.finish().expect_err("malformed JSON arguments should fail to parse");
        assert!(err.to_string().contains("foo"));
    }
}

//! Phase 5 M20: `GoogleProvider` — a real implementation against Gemini's
//! documented `generateContent`/`streamGenerateContent` REST API (built the
//! same spec-first way `anthropic_client.rs`/`openai.rs` were — no live key
//! to verify against here; see "What's unverified" below).
//!
//! Endpoints: `POST
//! https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key=<key>`
//! (non-streaming) and the same path with `:streamGenerateContent?alt=sse&key=<key>`
//! (streaming — Gemini's own REST streaming is "SSE of full/partial
//! `GenerateContentResponse` JSON objects", which happens to be exactly the
//! `data: {...}\n\n` shape `anthropic_client::SseDecoder` already parses, so
//! this reuses that decoder too, not a second one).
//!
//! ## Translating the shared (Anthropic-shaped) types
//!
//! Gemini's request shape (`contents: [{role, parts: [...]}]`,
//! `systemInstruction`, `tools: [{functionDeclarations: [...]}]`,
//! `toolConfig.functionCallingConfig` for forcing a call) differs from
//! Anthropic's in two structural ways this module's translation has to
//! bridge:
//!
//! - **Role names.** Gemini only recognizes `"user"`/`"model"`/`"function"`
//!   — an assistant turn becomes `"model"`, a plain user turn stays
//!   `"user"`, and a bundle of tool results becomes `"function"` (per
//!   Gemini's function-calling guide).
//! - **No call id.** Anthropic/OpenAI tool calls carry an opaque id the
//!   result is correlated back by; Gemini correlates purely by function
//!   *name* (a `functionResponse.name` must match the `functionCall.name`
//!   it answers, no id at all). Since this crate's shared
//!   [`ContentBlockParam::ToolResult`] only carries a `tool_use_id`, this
//!   module resolves that id back to the name it was originally returned
//!   with ([`collect_tool_call_names`], scanning every prior
//!   [`ContentBlockParam::ToolUse`] in the conversation — which, for a
//!   Gemini-driven conversation, this provider itself produced, so the
//!   mapping is always present). In the reverse direction, since Gemini's
//!   own `functionCall` has no id to echo back, this provider synthesizes
//!   one (`call_<n>`, in order of appearance) purely so the shared
//!   [`AssistantContentBlock::ToolUse`] shape (which does require an id) has
//!   something to put there — never sent to Gemini itself, only used
//!   internally by [`collect_tool_call_names`] on the next turn.
//!
//! ## What's unverified without a live key
//!
//! - Whether a real Gemini stream ever fragments one `functionCall`'s `args`
//!   across multiple chunks the way Anthropic/OpenAI fragment tool
//!   arguments. Gemini's documented behavior (and every example in its
//!   function-calling guide) shows a `functionCall` part arriving whole in
//!   one chunk, so this accumulator treats it that way — if a live response
//!   ever did fragment it, this would need the same partial-JSON-string
//!   accumulation `anthropic_client`'s `input_json_delta` handling uses.
//! - The exact `role` Gemini expects for a function-result turn —
//!   documented as `"function"` here, but Google's own examples have used
//!   `"user"` in some SDK versions; unverified against a live account.
//! - Whether `usageMetadata` appears on every streamed chunk (cumulative) or
//!   only the last — this accumulator just keeps the most recent value it
//!   sees either way, so it's correct under both.

use std::collections::HashMap;

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

const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

// ---------------------------------------------------------------------
// Shared-type -> Gemini wire-format translation.
// ---------------------------------------------------------------------

/// Maps every `ToolUse` block's id to its name across the whole
/// conversation so far — see this module's own docs for why a
/// `ToolResult`'s `tool_use_id` needs resolving back to a name for Gemini's
/// `functionResponse.name`.
fn collect_tool_call_names(messages: &[MessageParam]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        for block in &message.content {
            if let ContentBlockParam::ToolUse { id, name, .. } = block {
                names.insert(id.clone(), name.clone());
            }
        }
    }
    names
}

fn to_gemini_contents(messages: &[MessageParam]) -> Vec<Value> {
    let tool_call_names = collect_tool_call_names(messages);
    let mut out = Vec::new();

    for message in messages {
        if message.role == "assistant" {
            let mut parts = Vec::new();
            for block in &message.content {
                match block {
                    ContentBlockParam::Text { text } => parts.push(json!({ "text": text })),
                    ContentBlockParam::ToolUse { name, input, .. } => {
                        parts.push(json!({ "functionCall": { "name": name, "args": input } }));
                    }
                    ContentBlockParam::ToolResult { .. } => {}
                }
            }
            out.push(json!({ "role": "model", "parts": parts }));
            continue;
        }

        // A "user" `MessageParam`: plain text stays `role: "user"`; a
        // bundle of tool results becomes one `role: "function"` turn with
        // one `functionResponse` part per result (Gemini, unlike
        // OpenAI, keeps multiple function responses together in one turn
        // rather than requiring separate messages).
        let has_tool_results = message.content.iter().any(|b| matches!(b, ContentBlockParam::ToolResult { .. }));
        if has_tool_results {
            let parts: Vec<Value> = message
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlockParam::ToolResult { tool_use_id, content, is_error } => {
                        let name = tool_call_names.get(tool_use_id).cloned().unwrap_or_else(|| tool_use_id.clone());
                        let rendered = if *is_error { format!("Error: {content}") } else { content.clone() };
                        Some(json!({ "functionResponse": { "name": name, "response": { "content": rendered } } }))
                    }
                    _ => None,
                })
                .collect();
            out.push(json!({ "role": "function", "parts": parts }));
        } else {
            let parts: Vec<Value> =
                message.content.iter().filter_map(|b| match b { ContentBlockParam::Text { text } => Some(json!({ "text": text })), _ => None }).collect();
            out.push(json!({ "role": "user", "parts": parts }));
        }
    }

    out
}

fn to_gemini_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    let declarations: Vec<Value> =
        tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "parameters": t.input_schema })).collect();
    vec![json!({ "functionDeclarations": declarations })]
}

// ---------------------------------------------------------------------
// Response shape (shared between the non-streaming body and each streamed
// chunk — both are a `GenerateContentResponse`).
// ---------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct FunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    function_call: Option<FunctionCall>,
}

#[derive(Debug, Default, Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    #[serde(default)]
    content: Content,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: i64,
    #[serde(default)]
    candidates_token_count: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
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

/// Converts one candidate's parts into shared content blocks, synthesizing
/// a `call_<n>` id for each `functionCall` part (see this module's own docs
/// for why Gemini itself never gives one). `next_call_index` is threaded
/// through (rather than a fresh counter per call) so the streaming
/// accumulator can keep numbering calls consistently across chunks.
fn parts_to_content_blocks(parts: &[Part], next_call_index: &mut usize) -> Vec<AssistantContentBlock> {
    let mut content = Vec::new();
    for part in parts {
        if let Some(text) = &part.text {
            if !text.is_empty() {
                content.push(AssistantContentBlock::Text(text.clone()));
            }
        }
        if let Some(call) = &part.function_call {
            let id = format!("call_{next_call_index}");
            *next_call_index += 1;
            content.push(AssistantContentBlock::ToolUse { id, name: call.name.clone(), input: call.args.clone() });
        }
    }
    content
}

fn parse_non_stream_response(bytes: &[u8]) -> AppResult<AssistantTurn> {
    let parsed: GenerateContentResponse = serde_json::from_slice(bytes)
        .map_err(|e| AppError::Other(format!("failed to parse Gemini response: {e} (raw: {})", String::from_utf8_lossy(bytes))))?;

    let candidate = parsed.candidates.into_iter().next().ok_or_else(|| AppError::Other("Gemini response had no candidates".to_string()))?;

    let mut next_call_index = 0usize;
    // Merge consecutive text parts into one block, matching
    // `anthropic_client`'s own "one Text block per contiguous run of text"
    // shape rather than one block per part.
    let blocks = parts_to_content_blocks(&candidate.content.parts, &mut next_call_index);
    let content = merge_consecutive_text_blocks(blocks);

    let usage = parsed.usage_metadata.unwrap_or_default();
    Ok(AssistantTurn {
        content,
        stop_reason: candidate.finish_reason,
        usage: TurnUsage { input_tokens: usage.prompt_token_count, output_tokens: usage.candidates_token_count },
    })
}

fn merge_consecutive_text_blocks(blocks: Vec<AssistantContentBlock>) -> Vec<AssistantContentBlock> {
    let mut merged: Vec<AssistantContentBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        match (merged.last_mut(), &block) {
            (Some(AssistantContentBlock::Text(existing)), AssistantContentBlock::Text(new_text)) => existing.push_str(new_text),
            _ => merged.push(block),
        }
    }
    merged
}

// ---------------------------------------------------------------------
// Streaming accumulator.
// ---------------------------------------------------------------------

#[derive(Default)]
struct GeminiStreamAccumulator {
    text: String,
    tool_calls: Vec<(String, Value)>,
    finish_reason: Option<String>,
    usage: TurnUsage,
    next_call_index: usize,
}

impl GeminiStreamAccumulator {
    fn apply(&mut self, raw: &str, on_text_delta: &mut dyn FnMut(&str)) -> AppResult<()> {
        let chunk: GenerateContentResponse =
            serde_json::from_str(raw).map_err(|e| AppError::Other(format!("failed to parse Gemini stream chunk: {e} (raw: {raw})")))?;

        if let Some(usage) = chunk.usage_metadata {
            self.usage = TurnUsage { input_tokens: usage.prompt_token_count, output_tokens: usage.candidates_token_count };
        }

        for candidate in chunk.candidates {
            if let Some(reason) = candidate.finish_reason {
                self.finish_reason = Some(reason);
            }
            for part in &candidate.content.parts {
                if let Some(text) = &part.text {
                    if !text.is_empty() {
                        on_text_delta(text);
                        self.text.push_str(text);
                    }
                }
                if let Some(call) = &part.function_call {
                    let id = format!("call_{}", self.next_call_index);
                    self.next_call_index += 1;
                    self.tool_calls.push((id, json!({ "__name": call.name, "__args": call.args })));
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
        for (id, wrapped) in self.tool_calls {
            let name = wrapped["__name"].as_str().unwrap_or_default().to_string();
            let input = wrapped["__args"].clone();
            content.push(AssistantContentBlock::ToolUse { id, name, input });
        }
        Ok(AssistantTurn { content, stop_reason: self.finish_reason, usage: self.usage })
    }
}

// ---------------------------------------------------------------------
// HTTP layer.
// ---------------------------------------------------------------------

fn build_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder().use_rustls_tls().build().map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))
}

fn build_request_body(system: &str, messages: &[MessageParam], tools: &[ToolDefinition], max_tokens: u32, forced_tool: Option<&str>) -> Value {
    let mut body = json!({
        "contents": to_gemini_contents(messages),
        "systemInstruction": { "parts": [{ "text": system }] },
        "generationConfig": { "maxOutputTokens": max_tokens },
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(to_gemini_tools(tools));
    }
    if let Some(name) = forced_tool {
        body["toolConfig"] = json!({ "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": [name] } });
    }
    body
}

async fn send_and_check(
    http: &reqwest::Client,
    url: &str,
    body: Value,
    cancel: &CancellationToken,
) -> AppResult<Option<reqwest::Response>> {
    let send_fut = http.post(url).header("content-type", "application/json").json(&body).send();
    let response = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(None),
        result = send_fut => result.map_err(|e| AppError::Other(format!("Gemini API request failed: {e}")))?,
    };
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<ApiErrorBody>(&text).map(|b| b.error.message).unwrap_or(text);
        return Err(AppError::Other(format!("Gemini API returned HTTP {status}: {detail}")));
    }
    Ok(Some(response))
}

// ---------------------------------------------------------------------
// The provider itself.
// ---------------------------------------------------------------------

pub struct GoogleProvider {
    http: reqwest::Client,
    api_key: String,
}

impl GoogleProvider {
    pub fn new(api_key: String) -> AppResult<Self> {
        Ok(Self { http: build_http_client()?, api_key })
    }
}

#[async_trait]
impl ModelProvider for GoogleProvider {
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
        let url = format!("{API_BASE}/{model}:streamGenerateContent?alt=sse&key={}", self.api_key);
        let body = build_request_body(system, messages, tools, max_tokens, None);

        let Some(response) = send_and_check(&self.http, &url, body, cancel).await? else {
            return Ok(StreamOutcome::Cancelled);
        };

        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut accumulator = GeminiStreamAccumulator::default();

        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(StreamOutcome::Cancelled),
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = next else { break };
            let bytes = chunk.map_err(|e| AppError::Other(format!("Gemini API stream read failed: {e}")))?;
            for event in decoder.feed(&bytes) {
                accumulator.apply(&event.data, on_text_delta)?;
            }
        }

        Ok(StreamOutcome::Turn(accumulator.finish()?))
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
        let url = format!("{API_BASE}/{model}:generateContent?key={}", self.api_key);
        let body = build_request_body(system, messages, std::slice::from_ref(tool), max_tokens, Some(&tool.name));

        let Some(response) = send_and_check(&self.http, &url, body, cancel).await? else {
            return Ok(StreamOutcome::Cancelled);
        };
        let body_fut = response.bytes();
        let bytes = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(StreamOutcome::Cancelled),
            result = body_fut => result.map_err(|e| AppError::Other(format!("Gemini API response read failed: {e}")))?,
        };

        Ok(StreamOutcome::Turn(parse_non_stream_response(&bytes)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse_data(value: &Value) -> String {
        format!("data: {value}\n\n")
    }

    // -- to_gemini_contents --------------------------------------------------

    #[test]
    fn translates_plain_user_text() {
        let messages = vec![MessageParam::user_text("hello")];
        let out = to_gemini_contents(&messages);
        assert_eq!(out, vec![json!({ "role": "user", "parts": [{ "text": "hello" }] })]);
    }

    #[test]
    fn translates_an_assistant_turn_with_text_and_a_tool_use() {
        let messages = vec![MessageParam::assistant(vec![
            ContentBlockParam::Text { text: "Checking.".to_string() },
            ContentBlockParam::ToolUse { id: "toolu_1".to_string(), name: "read_file".to_string(), input: json!({"path": "a.rs"}) },
        ])];
        let out = to_gemini_contents(&messages);
        assert_eq!(out[0]["role"], "model");
        assert_eq!(out[0]["parts"][0]["text"], "Checking.");
        assert_eq!(out[0]["parts"][1]["functionCall"]["name"], "read_file");
        assert_eq!(out[0]["parts"][1]["functionCall"]["args"], json!({"path": "a.rs"}));
    }

    #[test]
    fn resolves_tool_result_back_to_the_function_name_it_answers() {
        let messages = vec![
            MessageParam::assistant(vec![ContentBlockParam::ToolUse {
                id: "toolu_1".to_string(),
                name: "read_file".to_string(),
                input: json!({}),
            }]),
            MessageParam::user_tool_results(vec![ContentBlockParam::ToolResult {
                tool_use_id: "toolu_1".to_string(),
                content: "file contents".to_string(),
                is_error: false,
            }]),
        ];
        let out = to_gemini_contents(&messages);
        assert_eq!(out[1]["role"], "function");
        assert_eq!(out[1]["parts"][0]["functionResponse"]["name"], "read_file");
        assert_eq!(out[1]["parts"][0]["functionResponse"]["response"]["content"], "file contents");
    }

    #[test]
    fn an_unresolvable_tool_result_id_falls_back_to_the_id_itself_rather_than_erroring() {
        let messages = vec![MessageParam::user_tool_results(vec![ContentBlockParam::ToolResult {
            tool_use_id: "unknown_id".to_string(),
            content: "x".to_string(),
            is_error: false,
        }])];
        let out = to_gemini_contents(&messages);
        assert_eq!(out[0]["parts"][0]["functionResponse"]["name"], "unknown_id");
    }

    // -- parse_non_stream_response -------------------------------------------

    #[test]
    fn parses_a_non_stream_response_with_a_forced_function_call() {
        let raw = json!({
            "candidates": [{
                "content": { "parts": [{ "functionCall": { "name": "propose_plan", "args": { "tasks": [] } } }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": { "promptTokenCount": 50, "candidatesTokenCount": 10 }
        });
        let turn = parse_non_stream_response(raw.to_string().as_bytes()).expect("should parse");
        assert_eq!(turn.content.len(), 1);
        assert_eq!(
            turn.content[0],
            AssistantContentBlock::ToolUse { id: "call_0".to_string(), name: "propose_plan".to_string(), input: json!({"tasks": []}) }
        );
        assert_eq!(turn.usage, TurnUsage { input_tokens: 50, output_tokens: 10 });
    }

    #[test]
    fn merges_consecutive_text_parts_into_one_text_block() {
        let raw = json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Hello, " }, { "text": "world." }] },
                "finishReason": "STOP"
            }]
        });
        let turn = parse_non_stream_response(raw.to_string().as_bytes()).expect("should parse");
        assert_eq!(turn.content, vec![AssistantContentBlock::Text("Hello, world.".to_string())]);
    }

    // -- streaming accumulator -----------------------------------------------

    #[test]
    fn accumulates_incremental_text_chunks_and_a_whole_function_call() {
        let mut raw = String::new();
        raw += &sse_data(&json!({ "candidates": [{ "content": { "parts": [{ "text": "Hello" }] } }] }));
        raw += &sse_data(&json!({ "candidates": [{ "content": { "parts": [{ "text": ", world." }] } }] }));
        raw += &sse_data(&json!({
            "candidates": [{ "content": { "parts": [{ "functionCall": { "name": "run_tests", "args": {} } }] }, "finishReason": "STOP" }],
            "usageMetadata": { "promptTokenCount": 30, "candidatesTokenCount": 7 }
        }));

        let mut decoder = SseDecoder::new();
        let events = decoder.feed(raw.as_bytes());
        let mut acc = GeminiStreamAccumulator::default();
        let mut deltas = Vec::new();
        for event in &events {
            acc.apply(&event.data, &mut |t: &str| deltas.push(t.to_string())).expect("apply should succeed");
        }
        let turn = acc.finish().expect("finish should succeed");

        assert_eq!(deltas, vec!["Hello".to_string(), ", world.".to_string()]);
        assert_eq!(turn.content.len(), 2);
        assert_eq!(turn.content[0], AssistantContentBlock::Text("Hello, world.".to_string()));
        assert_eq!(
            turn.content[1],
            AssistantContentBlock::ToolUse { id: "call_0".to_string(), name: "run_tests".to_string(), input: json!({}) }
        );
        assert_eq!(turn.stop_reason.as_deref(), Some("STOP"));
        assert_eq!(turn.usage, TurnUsage { input_tokens: 30, output_tokens: 7 });
    }

    #[test]
    fn decoder_reuse_handles_a_chunk_split_across_feed_calls() {
        // Exercises `SseDecoder` directly against Gemini's own shape, same
        // as `anthropic_client`'s own decoder-buffering test — this
        // provider deliberately reuses that decoder rather than writing a
        // second one.
        let mut decoder = SseDecoder::new();
        let first = "data: {\"candidates\": [{\"content\": {\"pa";
        let second = "rts\": [{\"text\": \"hi\"}]}}]}\n\n";
        assert!(decoder.feed(first.as_bytes()).is_empty());
        let events = decoder.feed(second.as_bytes());
        assert_eq!(events.len(), 1);
    }
}

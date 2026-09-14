//! A minimal client for the Anthropic Messages API
//! (`POST https://api.anthropic.com/v1/messages`, `stream: true`), built
//! from the documented request/response/streaming-event shapes rather than
//! recalled from memory — this environment has no live API key to verify
//! against, so getting the wire format exactly right from the spec (and
//! unit-testing the tricky part, SSE tool-input accumulation, against
//! hand-built fixtures) is the only verification available. See the M6
//! final report for exactly what remains unverified against a live key.
//!
//! No SDK crate is used (there is no official Rust SDK for the Anthropic
//! API to reach for) — this hand-rolls the HTTP request and a small SSE
//! line-splitter over `reqwest`'s streaming body, since `eventsource-stream`
//! isn't already a dependency of this project.

use std::collections::HashMap;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

// ---------------------------------------------------------------------
// Request-side types
// ---------------------------------------------------------------------

/// One tool definition in the request's `tools` array:
/// `{"name": ..., "description": ..., "input_schema": <JSON Schema>}`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// One content block inside a request message. `is_error` is always
/// serialized (rather than omitted when `false`) — the API defaults it to
/// `false` either way, and always emitting it keeps this type simple.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlockParam {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageParam {
    pub role: String,
    pub content: Vec<ContentBlockParam>,
}

impl MessageParam {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: vec![ContentBlockParam::Text { text: text.into() }],
        }
    }

    pub fn assistant(content: Vec<ContentBlockParam>) -> Self {
        Self { role: "assistant".to_string(), content }
    }

    pub fn user_tool_results(results: Vec<ContentBlockParam>) -> Self {
        Self { role: "user".to_string(), content: results }
    }
}

#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: &'a [MessageParam],
    tools: &'a [ToolDefinition],
    stream: bool,
}

// ---------------------------------------------------------------------
// Response-side types: one fully-materialized assistant turn
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssistantContentBlock {
    Text(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantTurn {
    pub content: Vec<AssistantContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: TurnUsage,
}

impl AssistantTurn {
    pub fn tool_uses(&self) -> impl Iterator<Item = (&str, &str, &serde_json::Value)> {
        self.content.iter().filter_map(|b| match b {
            AssistantContentBlock::ToolUse { id, name, input } => {
                Some((id.as_str(), name.as_str(), input))
            }
            AssistantContentBlock::Text(_) => None,
        })
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                AssistantContentBlock::Text(t) => Some(t.as_str()),
                AssistantContentBlock::ToolUse { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Outcome of one streamed request: either a fully-materialized turn, or
/// `Cancelled` if the caller's `CancellationToken` fired before/during the
/// stream — distinguished from an `Err` because cancellation is an expected,
/// clean stop, not a failure to report as a run error.
#[derive(Debug)]
pub enum StreamOutcome {
    Turn(AssistantTurn),
    Cancelled,
}

// ---------------------------------------------------------------------
// SSE line-splitter
// ---------------------------------------------------------------------

/// One decoded Server-Sent Event: the `event: <name>` line's value (if any)
/// and the concatenated `data: <line>` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Incrementally splits a raw `text/event-stream` byte stream into complete
/// events. Anthropic's stream is single-`data:`-line-per-event in practice,
/// but this still joins multiple `data:` lines the spec allows (with `\n`)
/// for robustness, and correctly buffers a chunk boundary that lands
/// mid-event (network reads don't align with SSE event boundaries).
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: String,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds another chunk of bytes (assumed UTF-8, which the Messages API's
    /// `text/event-stream` body always is) and returns every event whose
    /// terminating blank line has now been seen. Anything after the last
    /// blank line stays buffered for the next call.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        // Normalize CRLF so a blank-line split works the same regardless of
        // which line ending the server used.
        if self.buffer.contains('\r') {
            self.buffer = self.buffer.replace("\r\n", "\n");
        }

        let mut events = Vec::new();
        loop {
            let Some(blank_at) = self.buffer.find("\n\n") else { break };
            let raw_event: String = self.buffer.drain(..blank_at + 2).collect();
            if let Some(event) = parse_one_event(&raw_event) {
                events.push(event);
            }
        }
        events
    }
}

fn parse_one_event(raw: &str) -> Option<SseEvent> {
    let mut event_name: Option<String> = None;
    let mut data_lines: Vec<&str> = Vec::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
        // ":"-prefixed comment lines and other fields (id:, retry:) carry no
        // data this client needs.
    }

    if data_lines.is_empty() {
        return None;
    }
    Some(SseEvent { event: event_name, data: data_lines.join("\n") })
}

// ---------------------------------------------------------------------
// Streaming event JSON shapes (the `data:` payload's `"type"` field)
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct UsageData {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct MessageStartInner {
    usage: UsageData,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlockStart {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    // A model turn can also contain e.g. `thinking` blocks; anything not
    // recognized is skipped as a block this client doesn't act on rather
    // than a parse failure.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlockDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaInner {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorData {
    #[serde(default)]
    message: String,
    #[serde(default = "default_error_type")]
    r#type: String,
}

fn default_error_type() -> String {
    "unknown_error".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent {
    MessageStart { message: MessageStartInner },
    ContentBlockStart { index: usize, content_block: ContentBlockStart },
    ContentBlockDelta { index: usize, delta: ContentBlockDelta },
    ContentBlockStop { index: usize },
    MessageDelta { delta: MessageDeltaInner, #[serde(default)] usage: Option<UsageData> },
    MessageStop {},
    Ping {},
    Error { error: ErrorData },
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------
// Accumulator: turns a sequence of `StreamEvent`s into one `AssistantTurn`
// ---------------------------------------------------------------------

enum InProgressBlock {
    Text(String),
    ToolUse { id: String, name: String, partial_json: String },
}

#[derive(Default)]
struct StreamAccumulator {
    blocks: HashMap<usize, InProgressBlock>,
    stop_reason: Option<String>,
    usage: TurnUsage,
}

impl StreamAccumulator {
    /// Applies one decoded SSE event's JSON `data` payload, calling
    /// `on_text_delta` for every streamed text fragment (the live "typing"
    /// feed — ephemeral, not persisted). Returns `Err` only for a genuine
    /// server-reported `error` event or malformed JSON; everything else
    /// (unrecognized block/delta types) is skipped rather than failing the
    /// whole stream, since Anthropic can add new block types Claude Code
    /// doesn't need to act on.
    fn apply(&mut self, raw_data: &str, on_text_delta: &mut dyn FnMut(&str)) -> AppResult<()> {
        let event: StreamEvent = serde_json::from_str(raw_data)
            .map_err(|e| AppError::Other(format!("failed to parse Anthropic stream event: {e} (raw: {raw_data})")))?;

        match event {
            StreamEvent::MessageStart { message } => {
                self.usage.input_tokens = message.usage.input_tokens;
                self.usage.output_tokens = message.usage.output_tokens;
            }
            StreamEvent::ContentBlockStart { index, content_block } => {
                let block = match content_block {
                    ContentBlockStart::Text { text } => InProgressBlock::Text(text),
                    ContentBlockStart::ToolUse { id, name, .. } => {
                        InProgressBlock::ToolUse { id, name, partial_json: String::new() }
                    }
                    ContentBlockStart::Unknown => return Ok(()),
                };
                self.blocks.insert(index, block);
            }
            StreamEvent::ContentBlockDelta { index, delta } => match delta {
                ContentBlockDelta::TextDelta { text } => {
                    on_text_delta(&text);
                    if let Some(InProgressBlock::Text(existing)) = self.blocks.get_mut(&index) {
                        existing.push_str(&text);
                    }
                }
                ContentBlockDelta::InputJsonDelta { partial_json } => {
                    if let Some(InProgressBlock::ToolUse { partial_json: existing, .. }) =
                        self.blocks.get_mut(&index)
                    {
                        existing.push_str(&partial_json);
                    }
                }
                ContentBlockDelta::Unknown => {}
            },
            StreamEvent::ContentBlockStop { .. } => {
                // Nothing to do: the block is finalized in `finish()`, once
                // every index's deltas are known to be complete. Keeping
                // finalization there (rather than parsing `partial_json` to
                // JSON here) means a block never needs to be re-entered.
            }
            StreamEvent::MessageDelta { delta, usage } => {
                if let Some(reason) = delta.stop_reason {
                    self.stop_reason = Some(reason);
                }
                if let Some(usage) = usage {
                    // message_delta's usage is cumulative for the turn, so
                    // this replaces rather than adds.
                    self.usage.output_tokens = usage.output_tokens;
                    if usage.input_tokens != 0 {
                        self.usage.input_tokens = usage.input_tokens;
                    }
                }
            }
            StreamEvent::MessageStop {} | StreamEvent::Ping {} => {}
            StreamEvent::Error { error } => {
                return Err(AppError::Other(format!(
                    "Anthropic API stream error ({}): {}",
                    error.r#type, error.message
                )));
            }
            StreamEvent::Unknown => {}
        }
        Ok(())
    }

    fn finish(self) -> AppResult<AssistantTurn> {
        let mut indices: Vec<usize> = self.blocks.keys().copied().collect();
        indices.sort_unstable();

        let mut content = Vec::with_capacity(indices.len());
        for index in indices {
            let block = self.blocks.get(&index).expect("index came from blocks.keys()");
            let materialized = match block {
                InProgressBlock::Text(text) => AssistantContentBlock::Text(text.clone()),
                InProgressBlock::ToolUse { id, name, partial_json } => {
                    let input = if partial_json.trim().is_empty() {
                        serde_json::Value::Object(Default::default())
                    } else {
                        serde_json::from_str(partial_json).map_err(|e| {
                            AppError::Other(format!(
                                "tool_use block '{name}' ({id}) produced invalid JSON input across its \
                                 input_json_delta chunks: {e} (accumulated: {partial_json})"
                            ))
                        })?
                    };
                    AssistantContentBlock::ToolUse { id: id.clone(), name: name.clone(), input }
                }
            };
            content.push(materialized);
        }

        Ok(AssistantTurn { content, stop_reason: self.stop_reason, usage: self.usage })
    }
}

// ---------------------------------------------------------------------
// The client itself
// ---------------------------------------------------------------------

pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: String,
}

impl AnthropicClient {
    pub fn new(api_key: String) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))?;
        Ok(Self { http, api_key })
    }

    /// Streams one assistant turn for `messages`/`tools`, calling
    /// `on_text_delta` for every streamed text fragment as it arrives (for a
    /// live "typing" UI — not persisted). Cancellation is checked before the
    /// request is sent and on every chunk read of the response body; in
    /// either case the in-flight `reqwest::Response`/byte stream is dropped,
    /// which tears down the underlying HTTP connection rather than letting
    /// it run to completion in the background.
    pub async fn stream_turn(
        &self,
        model: &str,
        max_tokens: u32,
        system: &str,
        messages: &[MessageParam],
        tools: &[ToolDefinition],
        cancel: &CancellationToken,
        mut on_text_delta: impl FnMut(&str),
    ) -> AppResult<StreamOutcome> {
        let body = MessagesRequest { model, max_tokens, system, messages, tools, stream: true };

        let send_fut = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send();

        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(StreamOutcome::Cancelled),
            result = send_fut => result.map_err(|e| AppError::Other(format!("Anthropic API request failed: {e}")))?,
        };

        let status = response.status();
        if !status.is_success() {
            // Never log/include the API key (it's not in the body or a
            // header we echo back) — just the server's own error body, which
            // is the actually useful diagnostic here.
            let text = response.text().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Anthropic API returned HTTP {status}: {text}"
            )));
        }

        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut accumulator = StreamAccumulator::default();

        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(StreamOutcome::Cancelled),
                chunk = stream.next() => chunk,
            };

            let Some(chunk) = next else { break };
            let bytes = chunk.map_err(|e| AppError::Other(format!("Anthropic API stream read failed: {e}")))?;

            for event in decoder.feed(&bytes) {
                accumulator.apply(&event.data, &mut on_text_delta)?;
            }
        }

        Ok(StreamOutcome::Turn(accumulator.finish()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse(event: &str, data: &serde_json::Value) -> String {
        format!("event: {event}\ndata: {}\n\n", data)
    }

    /// The trickiest part of the real wire format: a `tool_use` block's
    /// `input` arrives as a sequence of raw JSON string fragments
    /// (`input_json_delta.partial_json`) that must be concatenated in order
    /// and parsed as one JSON value only once `content_block_stop` fires —
    /// never parsed incrementally. This fixture mirrors a real streamed
    /// response containing one text block followed by one tool_use block
    /// (a fake `read_file` call) split across three `input_json_delta`
    /// chunks, plus the surrounding `message_start`/`message_delta` usage.
    #[test]
    fn accumulates_text_and_split_tool_input_into_one_turn() {
        let mut raw = String::new();
        raw += &sse("message_start", &serde_json::json!({
            "type": "message_start",
            "message": {"usage": {"input_tokens": 25, "output_tokens": 1}}
        }));
        raw += &sse("content_block_start", &serde_json::json!({
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}
        }));
        raw += &sse("content_block_delta", &serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "Let me check that file."}
        }));
        raw += &sse("content_block_stop", &serde_json::json!({"type": "content_block_stop", "index": 0}));
        raw += &sse("content_block_start", &serde_json::json!({
            "type": "content_block_start", "index": 1,
            "content_block": {"type": "tool_use", "id": "toolu_01AbC", "name": "read_file", "input": {}}
        }));
        raw += &sse("content_block_delta", &serde_json::json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "{\"pa"}
        }));
        raw += &sse("content_block_delta", &serde_json::json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "th\": \"src/ma"}
        }));
        raw += &sse("content_block_delta", &serde_json::json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "in.rs\"}"}
        }));
        raw += &sse("content_block_stop", &serde_json::json!({"type": "content_block_stop", "index": 1}));
        raw += &sse("message_delta", &serde_json::json!({
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use", "stop_sequence": null},
            "usage": {"output_tokens": 57}
        }));
        raw += &sse("message_stop", &serde_json::json!({"type": "message_stop"}));

        let mut decoder = SseDecoder::new();
        let events = decoder.feed(raw.as_bytes());
        assert_eq!(events.len(), 9, "should decode all 9 SSE events in one feed");

        let mut acc = StreamAccumulator::default();
        let mut deltas = Vec::new();
        for event in &events {
            acc.apply(&event.data, &mut |t: &str| deltas.push(t.to_string())).expect("apply should succeed");
        }
        let turn = acc.finish().expect("finish should succeed");

        assert_eq!(deltas, vec!["Let me check that file.".to_string()]);
        assert_eq!(turn.content.len(), 2);
        assert_eq!(turn.content[0], AssistantContentBlock::Text("Let me check that file.".to_string()));
        assert_eq!(
            turn.content[1],
            AssistantContentBlock::ToolUse {
                id: "toolu_01AbC".to_string(),
                name: "read_file".to_string(),
                input: serde_json::json!({"path": "src/main.rs"}),
            }
        );
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(turn.usage, TurnUsage { input_tokens: 25, output_tokens: 57 });
    }

    #[test]
    fn tool_use_with_empty_input_across_no_deltas_finalizes_as_empty_object() {
        let mut raw = String::new();
        raw += &sse("content_block_start", &serde_json::json!({
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "tool_use", "id": "toolu_1", "name": "git_status", "input": {}}
        }));
        raw += &sse("content_block_stop", &serde_json::json!({"type": "content_block_stop", "index": 0}));

        let mut decoder = SseDecoder::new();
        let events = decoder.feed(raw.as_bytes());
        let mut acc = StreamAccumulator::default();
        for event in &events {
            acc.apply(&event.data, &mut |_: &str| {}).expect("apply should succeed");
        }
        let turn = acc.finish().expect("finish should succeed");
        assert_eq!(
            turn.content[0],
            AssistantContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "git_status".to_string(),
                input: serde_json::json!({}),
            }
        );
    }

    #[test]
    fn decoder_buffers_an_event_split_across_multiple_feed_calls() {
        let mut decoder = SseDecoder::new();
        let first_half = "event: content_block_delta\ndata: {\"type\": \"content_block_delta\", \"in";
        let second_half = "dex\": 0, \"delta\": {\"type\": \"text_delta\", \"text\": \"hi\"}}\n\n";

        let events = decoder.feed(first_half.as_bytes());
        assert!(events.is_empty(), "incomplete event should not be emitted yet");

        let events = decoder.feed(second_half.as_bytes());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));
    }

    #[test]
    fn decoder_handles_multiple_events_in_one_feed_and_leaves_partial_trailing_event_buffered() {
        let mut decoder = SseDecoder::new();
        let chunk = "event: ping\ndata: {\"type\": \"ping\"}\n\nevent: ping\ndata: {\"type\": \"ping\"}\n\nevent: message_stop\ndata: {\"type\": \"mess";
        let events = decoder.feed(chunk.as_bytes());
        assert_eq!(events.len(), 2, "only the two complete events should be emitted");

        let events = decoder.feed(b"age_stop\"}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"type\": \"message_stop\"}");
    }

    #[test]
    fn server_error_event_surfaces_as_an_error_not_a_panic() {
        let mut acc = StreamAccumulator::default();
        let raw = serde_json::json!({
            "type": "error",
            "error": {"type": "overloaded_error", "message": "Overloaded"}
        })
        .to_string();
        let err = acc.apply(&raw, &mut |_: &str| {}).expect_err("error event should surface as Err");
        assert!(err.to_string().contains("Overloaded"));
    }

    #[test]
    fn unrecognized_event_type_is_skipped_rather_than_failing() {
        let mut acc = StreamAccumulator::default();
        let raw = serde_json::json!({"type": "some_future_event_type", "whatever": 1}).to_string();
        acc.apply(&raw, &mut |_: &str| {}).expect("unknown event types should be skipped, not error");
    }
}

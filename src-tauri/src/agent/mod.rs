//! M6: the real Anthropic tool-calling loop. A queued `agent_runs` row
//! (created by `commands::agent_commands::start_worktree_for_agent`, M5)
//! becomes a running agent — a sequential loop that calls the Anthropic
//! Messages API, dispatches whatever tools it asks for inside the run's
//! isolated git worktree, and persists/streams everything until the run
//! ends in `completed`/`failed`/`stopped`.
//!
//! Module map:
//! - `anthropic_client` — the Messages API HTTP/SSE client
//! - `schema`           — the agent-facing tool definitions (JSON Schema)
//! - `path_guard`        — the workspace-escape guard every filesystem tool uses
//! - `process`           — shared child-process spawn/timeout/cancel, used by `tools` and (M12) `commands::testing_commands`
//! - `tools`             — dispatch for each tool by name
//! - `executor`          — runs one `tool_use` block, persisting/emitting its lifecycle
//! - `events`            — typed event-emission helpers shared by the loop and executor
//! - `tool_loop`         — `run_agent_loop`, the actual orchestration
//! - `test_fix`          — M13: bounded, visible tracking of the self-healing test-fix cycle
//! - `reviewer`          — M14: the read-only reviewer agent (`run_review`)
//! - `conflict_resolver` — M15: the bounded, scoped AI merge-conflict resolver

pub mod anthropic_client;
pub mod conflict_resolver;
pub mod events;
pub mod executor;
pub mod path_guard;
pub mod process;
pub mod reviewer;
pub mod schema;
pub mod test_fix;
pub mod tool_loop;
pub mod tools;

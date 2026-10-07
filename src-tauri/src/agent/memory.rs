//! Phase 5 M18: per-agent memory — extraction (at the end of a run) and
//! retrieval (at the start of a new one). Both halves are pure, DB-free
//! functions, deliberately kept that way so the whole thing is unit
//! testable without a live API key or a database (this environment has
//! neither in CI):
//!
//!   * [`extract_memories_from_run`]: mechanical extraction from data a run
//!     already produced — a `completed_work` entry from the run's own
//!     `report_completion` summary (or the weak-completion fallback text),
//!     an `error` entry from a real failure reason, and `file_context`
//!     entries derived straight from `tool_calls` rows
//!     (`write_file`/`edit_file`). No AI call: everything here is already
//!     sitting in data the run persisted, so a fresh structured-output call
//!     (the `brain`/`planner`/`reviewer` pattern) would just be re-deriving
//!     what's mechanically obtainable — this milestone's own guidance is to
//!     avoid that. There is deliberately no `decision`-kind extraction
//!     here: a genuine "why did the agent choose X over Y" summary would
//!     need real judgment calls an AI call *could* add, but nothing in this
//!     milestone's tool surface captures that signal distinctly from the
//!     completion summary already does, so inventing a second call to
//!     re-paraphrase the same text would be manufactured, not real, value.
//!
//!   * [`rank_relevant_memories`]: a simple, documented keyword-overlap
//!     retrieval heuristic over a new task prompt vs. stored
//!     `relevance_tags`/`content` — exact token matches outrank partial
//!     (substring) matches, ties break by recency, and the result is
//!     capped small. Anything scoring zero is dropped entirely rather than
//!     padded in, which is what keeps this genuinely selective instead of a
//!     full dump of an agent's history into every prompt.
//!
//! [`render_memory_context`] turns a retrieval result into the
//! clearly-labeled prompt section `agent::tool_loop::build_system_prompt`
//! appends.

use std::collections::HashSet;

use crate::db::models::{AgentMemory, AgentMemoryKind, AgentRunStatus, ToolCall, ToolCallStatus};

/// How many distinct files `extract_memories_from_run` will turn into
/// `file_context` entries for a single run — a real agent run usually
/// touches a handful of files; this is just a hard ceiling against a
/// pathological run that rewrote dozens of files, so a single run's
/// extraction never balloons into "dump everything" on its own.
const MAX_FILE_CONTEXT_ENTRIES: usize = 10;

/// The default cap [`rank_relevant_memories`]'s callers use — a small,
/// genuinely selective number of entries, never a wholesale dump. Exposed
/// as a constant (rather than inlined at the one real call site in
/// `agent::tool_loop`) so it's named and testable like
/// `brain::REGENERATE_AFTER_COMMITS` is.
pub const DEFAULT_RETRIEVAL_CAP: usize = 5;

/// Minimum token length kept by [`tokenize`] — short tokens (`"a"`, `"to"`,
/// `"is"`, ...) are either stopwords already or too generic to mean
/// anything as a keyword.
const MIN_TOKEN_LEN: usize = 3;

/// A deliberately small, generic English stopword list — just enough to
/// keep the most common connective words out of both stored tags and query
/// tokens, not a full NLP stopword corpus (this is keyword-overlap
/// retrieval, not search engine infrastructure).
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "has", "have", "her", "was",
    "one", "our", "out", "day", "get", "him", "his", "how", "man", "new", "now", "old", "see", "two", "way",
    "who", "boy", "did", "its", "let", "put", "say", "she", "too", "use", "with", "this", "that", "from",
    "they", "will", "been", "were", "then", "than", "when", "what", "into", "your", "some", "more", "does",
    "could", "would", "should", "about",
];

/// Splits `text` on any non-alphanumeric run, lowercases, drops anything
/// shorter than [`MIN_TOKEN_LEN`] or in [`STOPWORDS`], and deduplicates
/// while preserving first-seen order.
fn tokenize(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tokens = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let token = raw.to_lowercase();
        if token.len() < MIN_TOKEN_LEN || STOPWORDS.contains(&token.as_str()) {
            continue;
        }
        if seen.insert(token.clone()) {
            tokens.push(token);
        }
    }
    tokens
}

/// Public wrapper around [`tokenize`] for building a memory's own
/// `relevance_tags` at extraction time — a plain space-separated keyword
/// string (see `migrations/0011_agent_memory.sql` for why that
/// representation was chosen over a JSON array).
pub fn tags_from_text(text: &str) -> String {
    tokenize(text).join(" ")
}

fn token_set(text: &str) -> HashSet<String> {
    tokenize(text).into_iter().collect()
}

// ---------------------------------------------------------------------
// Extraction — pure, from data a run already produced.
// ---------------------------------------------------------------------

/// One memory entry [`extract_memories_from_run`] produced, ready to be
/// persisted via `db::repository::agent_memory::insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedMemory {
    pub kind: AgentMemoryKind,
    pub content: String,
    pub relevance_tags: String,
}

/// Scans `tool_calls` for successful `write_file`/`edit_file` calls and
/// returns the distinct file paths they touched, in first-touched order,
/// capped at [`MAX_FILE_CONTEXT_ENTRIES`]. A failed write/edit call didn't
/// actually change anything real, so it's excluded — this only reports
/// genuine, successful touches.
fn touched_file_paths(tool_calls: &[ToolCall]) -> Vec<String> {
    let mut paths = Vec::new();
    for call in tool_calls {
        if call.status != ToolCallStatus::Success {
            continue;
        }
        if call.tool_name != "write_file" && call.tool_name != "edit_file" {
            continue;
        }
        let Ok(input) = serde_json::from_str::<serde_json::Value>(&call.input_json) else { continue };
        let Some(path) = input.get("path").and_then(|p| p.as_str()) else { continue };
        if !paths.iter().any(|p: &String| p == path) {
            paths.push(path.to_string());
        }
        if paths.len() >= MAX_FILE_CONTEXT_ENTRIES {
            break;
        }
    }
    paths
}

/// Builds the small set of memory entries a just-finished run leaves
/// behind for its agent. `outcome_detail` and `error_message` are the same
/// two values `agent::tool_loop::finish_run` already computes for the
/// run's notification (the real `report_completion` summary, or the
/// genuine failure reason) — reused here rather than re-derived.
///
/// - `Completed`: one `completed_work` entry from `outcome_detail`, if
///   it's non-empty (the weak-completion path — the model stopping
///   without calling any tool — can leave this `None`, in which case
///   there's honestly nothing to summarize, so nothing is added).
/// - `Failed`: one `error` entry from `error_message` (falling back to
///   `outcome_detail` if `error_message` itself is empty).
/// - `Stopped`/`Queued`/`Running`: no completion/error entry — a stop is
///   usually a deliberate halt (user request, test-fix budget), not
///   itself a fact worth remembering as "work" or "an error".
/// - Every outcome: `file_context` entries for files genuinely touched,
///   regardless of how the run ended.
pub fn extract_memories_from_run(
    status: AgentRunStatus,
    outcome_detail: Option<&str>,
    error_message: Option<&str>,
    tool_calls: &[ToolCall],
) -> Vec<ExtractedMemory> {
    let mut memories = Vec::new();

    let non_empty = |s: Option<&str>| s.map(str::trim).filter(move |s| !s.is_empty());

    match status {
        AgentRunStatus::Completed => {
            if let Some(summary) = non_empty(outcome_detail) {
                memories.push(ExtractedMemory {
                    kind: AgentMemoryKind::CompletedWork,
                    content: summary.to_string(),
                    relevance_tags: tags_from_text(summary),
                });
            }
        }
        AgentRunStatus::Failed => {
            if let Some(reason) = non_empty(error_message).or_else(|| non_empty(outcome_detail)) {
                memories.push(ExtractedMemory {
                    kind: AgentMemoryKind::Error,
                    content: reason.to_string(),
                    relevance_tags: tags_from_text(reason),
                });
            }
        }
        AgentRunStatus::Stopped | AgentRunStatus::Queued | AgentRunStatus::Running => {}
    }

    for path in touched_file_paths(tool_calls) {
        memories.push(ExtractedMemory {
            relevance_tags: tags_from_text(&path),
            content: format!("Modified `{path}` during this run."),
            kind: AgentMemoryKind::FileContext,
        });
    }

    memories
}

// ---------------------------------------------------------------------
// Retrieval ranking — pure, over already-loaded `AgentMemory` rows.
// ---------------------------------------------------------------------

/// Exact token matches (weight 2) outrank partial/substring matches
/// (weight 1) — e.g. a query for "login" exactly matching a stored
/// "login" tag beats it merely overlapping with a stored "logging"/"log".
/// Returns 0 (never relevant) if either side tokenizes to nothing.
fn relevance_score(query_tokens: &HashSet<String>, memory: &AgentMemory) -> u32 {
    if query_tokens.is_empty() {
        return 0;
    }
    let memory_text = format!("{} {}", memory.relevance_tags, memory.content);
    let memory_tokens = token_set(&memory_text);
    if memory_tokens.is_empty() {
        return 0;
    }

    let mut exact = 0u32;
    let mut partial = 0u32;
    for q in query_tokens {
        if memory_tokens.contains(q) {
            exact += 1;
        } else if memory_tokens.iter().any(|m| m.contains(q.as_str()) || q.contains(m.as_str())) {
            partial += 1;
        }
    }
    exact * 2 + partial
}

/// Ranks `memories` by keyword overlap with `task_prompt`, drops anything
/// that scores zero (no real overlap at all — never padded in just to fill
/// the cap), breaks ties by recency (`created_at`, descending — ISO 8601
/// strings sort lexicographically the same as chronologically), and
/// returns at most `cap` entries. This is the one enforcement point for
/// "selective, not a full dump" — a task with no genuine keyword overlap
/// with anything in memory gets back an empty `Vec`.
pub fn rank_relevant_memories<'a>(task_prompt: &str, memories: &'a [AgentMemory], cap: usize) -> Vec<&'a AgentMemory> {
    let query_tokens = token_set(task_prompt);
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(u32, &AgentMemory)> =
        memories.iter().map(|m| (relevance_score(&query_tokens, m), m)).filter(|(score, _)| *score > 0).collect();

    scored.sort_by(|(score_a, mem_a), (score_b, mem_b)| score_b.cmp(score_a).then_with(|| mem_b.created_at.cmp(&mem_a.created_at)));

    scored.into_iter().take(cap).map(|(_, m)| m).collect()
}

fn kind_label(kind: AgentMemoryKind) -> &'static str {
    match kind {
        AgentMemoryKind::Decision => "Decision",
        AgentMemoryKind::FileContext => "Touched file",
        AgentMemoryKind::Error => "Past error",
        AgentMemoryKind::CompletedWork => "Completed work",
    }
}

/// Renders an already-ranked/capped retrieval result into the labeled
/// section `agent::tool_loop::build_system_prompt` appends to a new run's
/// system prompt — `None` when `memories` is empty, so a run with no real
/// overlap gets a prompt that's honestly shorter rather than an empty
/// "Relevant context" header followed by nothing.
pub fn render_memory_context(memories: &[&AgentMemory]) -> Option<String> {
    if memories.is_empty() {
        return None;
    }
    let mut section = String::from(
        "Relevant context from past work on this agent (retrieved because it overlaps with this task's own \
         wording — it may or may not still apply; use your own judgment):\n",
    );
    for memory in memories {
        section.push_str(&format!("- [{}] {}\n", kind_label(memory.kind), memory.content));
    }
    Some(section)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- tokenize / tags_from_text -------------------------------------

    #[test]
    fn tags_from_text_lowercases_splits_and_drops_stopwords_and_short_tokens() {
        let tags = tags_from_text("The Login Flow is a Big Fix");
        // "the"/"is"/"a" are dropped (stopword/too short); "big" and "fix"
        // survive at exactly MIN_TOKEN_LEN.
        assert_eq!(tags, "login flow big fix");
    }

    #[test]
    fn tags_from_text_splits_file_paths_on_every_separator() {
        assert_eq!(tags_from_text("src/auth.rs"), "src auth");
    }

    // -- extract_memories_from_run --------------------------------------

    fn tool_call(tool_name: &str, input_json: &str, status: ToolCallStatus) -> ToolCall {
        ToolCall {
            id: "tc1".to_string(),
            agent_run_id: "run1".to_string(),
            sequence_number: 1,
            tool_use_id: "toolu_1".to_string(),
            tool_name: tool_name.to_string(),
            input_json: input_json.to_string(),
            output_json: None,
            status,
            error_message: None,
            started_at: "2024-01-01T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
        }
    }

    #[test]
    fn completed_run_produces_a_completed_work_entry_and_file_context() {
        let calls = vec![tool_call("write_file", r#"{"path":"src/auth.rs","content":"..."}"#, ToolCallStatus::Success)];
        let memories = extract_memories_from_run(
            AgentRunStatus::Completed,
            Some("Fixed the login bug and added a test."),
            None,
            &calls,
        );
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0].kind, AgentMemoryKind::CompletedWork);
        assert_eq!(memories[0].content, "Fixed the login bug and added a test.");
        assert_eq!(memories[1].kind, AgentMemoryKind::FileContext);
        assert!(memories[1].content.contains("src/auth.rs"));
    }

    #[test]
    fn completed_run_with_no_real_summary_produces_no_completed_work_entry() {
        let memories = extract_memories_from_run(AgentRunStatus::Completed, None, None, &[]);
        assert!(memories.is_empty(), "nothing to fabricate a summary from — must stay honestly empty");
    }

    #[test]
    fn failed_run_uses_the_real_error_message_over_the_generic_detail() {
        let memories = extract_memories_from_run(
            AgentRunStatus::Failed,
            Some("some generic detail"),
            Some("Build failed: missing semicolon"),
            &[],
        );
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].kind, AgentMemoryKind::Error);
        assert_eq!(memories[0].content, "Build failed: missing semicolon");
    }

    #[test]
    fn failed_run_falls_back_to_detail_when_error_message_is_empty() {
        let memories = extract_memories_from_run(AgentRunStatus::Failed, Some("stopped for a real reason"), None, &[]);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "stopped for a real reason");
    }

    #[test]
    fn stopped_run_produces_no_completed_or_error_entry_but_still_extracts_file_context() {
        let calls = vec![tool_call("edit_file", r#"{"path":"src/b.rs","old_string":"a","new_string":"b"}"#, ToolCallStatus::Success)];
        let memories = extract_memories_from_run(AgentRunStatus::Stopped, Some("Stopped by user request."), None, &calls);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].kind, AgentMemoryKind::FileContext);
    }

    #[test]
    fn dedups_repeated_touches_of_the_same_file_into_one_entry() {
        let calls = vec![
            tool_call("write_file", r#"{"path":"src/a.rs","content":"1"}"#, ToolCallStatus::Success),
            tool_call("edit_file", r#"{"path":"src/a.rs","old_string":"1","new_string":"2"}"#, ToolCallStatus::Success),
            tool_call("write_file", r#"{"path":"src/b.rs","content":"1"}"#, ToolCallStatus::Success),
        ];
        let memories = extract_memories_from_run(AgentRunStatus::Completed, None, None, &calls);
        let file_entries: Vec<&ExtractedMemory> = memories.iter().filter(|m| m.kind == AgentMemoryKind::FileContext).collect();
        assert_eq!(file_entries.len(), 2);
        assert!(file_entries[0].content.contains("src/a.rs"));
        assert!(file_entries[1].content.contains("src/b.rs"));
    }

    #[test]
    fn ignores_failed_tool_calls_and_non_write_tools_when_deriving_file_context() {
        let calls = vec![
            tool_call("write_file", r#"{"path":"src/a.rs","content":"1"}"#, ToolCallStatus::Error),
            tool_call("read_file", r#"{"path":"src/b.rs"}"#, ToolCallStatus::Success),
        ];
        let memories = extract_memories_from_run(AgentRunStatus::Completed, None, None, &calls);
        assert!(memories.is_empty());
    }

    #[test]
    fn caps_file_context_entries_at_the_maximum() {
        let calls: Vec<ToolCall> = (0..15)
            .map(|i| tool_call("write_file", &format!(r#"{{"path":"src/file{i}.rs","content":"x"}}"#), ToolCallStatus::Success))
            .collect();
        let memories = extract_memories_from_run(AgentRunStatus::Completed, None, None, &calls);
        assert_eq!(memories.len(), MAX_FILE_CONTEXT_ENTRIES);
    }

    // -- rank_relevant_memories ------------------------------------------

    fn memory_fixture(content: &str, tags: &str, created_at: &str) -> AgentMemory {
        AgentMemory {
            id: format!("id-{content}-{created_at}"),
            agent_id: "a1".to_string(),
            agent_run_id: None,
            kind: AgentMemoryKind::Decision,
            content: content.to_string(),
            relevance_tags: tags.to_string(),
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn exact_keyword_match_ranks_above_partial_match() {
        let exact = memory_fixture("Implemented login flow correctly", "login flow", "2024-01-01T00:00:00Z");
        let partial = memory_fixture("debug logging output", "log", "2024-01-01T00:00:00Z");
        let memories = vec![partial.clone(), exact.clone()];

        let ranked = rank_relevant_memories("Fix the login flow bug", &memories, 5);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].content, exact.content, "the exact match should rank first");
        assert_eq!(ranked[1].content, partial.content);
    }

    #[test]
    fn recency_breaks_ties_when_scores_are_equal() {
        let old = memory_fixture("old decision about login", "login", "2024-01-01T00:00:00Z");
        let new = memory_fixture("new decision about login", "login", "2024-06-01T00:00:00Z");
        let memories = vec![old.clone(), new.clone()];

        let ranked = rank_relevant_memories("login", &memories, 5);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].content, new.content, "the more recent memory should win a tied score");
        assert_eq!(ranked[1].content, old.content);
    }

    #[test]
    fn caps_at_the_requested_limit_keeping_the_most_recent() {
        let days = ["01", "02", "03", "04", "05", "06", "07", "08"];
        let memories: Vec<AgentMemory> =
            days.iter().map(|d| memory_fixture("keyword decision", "keyword", &format!("2024-01-{d}T00:00:00Z"))).collect();

        let ranked = rank_relevant_memories("keyword", &memories, 5);

        assert_eq!(ranked.len(), 5);
        let ranked_days: Vec<&str> = ranked.iter().map(|m| &m.created_at[8..10]).collect();
        assert_eq!(ranked_days, vec!["08", "07", "06", "05", "04"]);
    }

    #[test]
    fn empty_when_nothing_relevant_matches() {
        let memories = vec![
            memory_fixture("Refactored the billing module", "billing module", "2024-01-01T00:00:00Z"),
            memory_fixture("Added caching layer", "caching layer", "2024-01-02T00:00:00Z"),
        ];

        let ranked = rank_relevant_memories("Investigate the frontend rendering glitch", &memories, 5);

        assert!(ranked.is_empty(), "no genuine keyword overlap should mean nothing is injected");
    }

    #[test]
    fn empty_memories_input_returns_empty() {
        let memories: Vec<AgentMemory> = vec![];
        assert!(rank_relevant_memories("anything at all", &memories, 5).is_empty());
    }

    #[test]
    fn query_with_only_stopwords_yields_no_results() {
        let memories = vec![memory_fixture("Implemented login flow", "login flow", "2024-01-01T00:00:00Z")];
        let ranked = rank_relevant_memories("the and for are", &memories, 5);
        assert!(ranked.is_empty());
    }

    // -- render_memory_context --------------------------------------------

    #[test]
    fn render_memory_context_is_none_for_an_empty_slice() {
        assert!(render_memory_context(&[]).is_none());
    }

    #[test]
    fn render_memory_context_labels_each_entry_by_kind() {
        let decision = memory_fixture("Chose SQLite over Postgres", "sqlite postgres", "2024-01-01T00:00:00Z");
        let mut error_entry = memory_fixture("Build failed once already", "build failed", "2024-01-02T00:00:00Z");
        error_entry.kind = AgentMemoryKind::Error;
        let refs = vec![&decision, &error_entry];

        let section = render_memory_context(&refs).expect("non-empty input should render Some");

        assert!(section.contains("Relevant context from past work"));
        assert!(section.contains("[Decision] Chose SQLite over Postgres"));
        assert!(section.contains("[Past error] Build failed once already"));
    }
}

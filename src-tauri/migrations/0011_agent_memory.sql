-- Phase 5 M18: per-agent persisted memory — real, selectively-retrieved
-- context from an agent's own past runs (decisions, files touched, errors
-- hit, completed work). Extracted mechanically from that run's own
-- `tool_calls` data, plus (for a successfully completed run) the agent's
-- own real `report_completion` summary — see `agent::memory::
-- extract_memories_from_run`. Never a second AI call purely to re-derive
-- data this run already produced.
--
-- Scoped to `agent_id`, not `project_id`: an agent here is a long-lived
-- identity, not a disposable per-mission task — the same `agent_id` is
-- reused across every "Start" run on it (see
-- `commands::agent_commands::start_worktree_for_agent`, which always takes
-- an existing `agent_id` and just creates a fresh `agent_runs` row/worktree
-- for it). So an agent's own run history is the natural, honest join for
-- its memory; pooling every agent's memory at the project level would leak
-- one agent's internal decisions/errors into another agent's retrieval
-- results for no real benefit.
--
-- `relevance_tags` is a plain space-separated lowercase keyword string, not
-- a JSON array: retrieval (`agent::memory::rank_relevant_memories`) only
-- ever needs a token set to intersect against the new run's task prompt, so
-- a JSON array would just mean parsing it back into the same token set on
-- every read for no benefit — a plain string is the simpler, equally
-- honest choice.
--
-- There is deliberately no stored `relevance_score` column, unlike the
-- plan's own literal suggestion: relevance is query-dependent (the same
-- memory is more or less relevant depending on the *next* task prompt), so
-- a static score computed once at write time would misrepresent it as a
-- fixed property of the memory. Scoring happens at retrieval time instead,
-- over `relevance_tags`/`content` against the new task prompt.
CREATE TABLE agent_memory (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  agent_run_id TEXT REFERENCES agent_runs(id) ON DELETE SET NULL,
  kind TEXT NOT NULL CHECK(kind IN ('decision', 'file_context', 'error', 'completed_work')),
  content TEXT NOT NULL,
  relevance_tags TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_agent_memory_agent_id ON agent_memory(agent_id);

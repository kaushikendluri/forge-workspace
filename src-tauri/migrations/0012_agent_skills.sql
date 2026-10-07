-- Phase 5 M19: Agent Skills — reusable, named specialist agent
-- configurations (e.g. "Senior React Engineer", "Security Auditor") a user
-- defines once (name/description/instructions/allowed tools/preferred
-- model) and selects when creating a new agent
-- (`commands::agent_commands::create_agent`'s optional `skill_id`), rather
-- than retyping the same system-prompt addendum and tool restrictions by
-- hand every time.
--
-- `tools_json` is a JSON array of strings: either the real tool names from
-- the catalogue `agent::schema::all_tool_definitions` returns, or the single
-- sentinel element `"*"` meaning "every tool, unrestricted" — exactly
-- today's default (no skill at all) behavior. `["*"]` and `[]` are both
-- valid and mean different things: `["*"]` is unrestricted, `[]` is no
-- tool access at all. Validated against the real catalogue at both create
-- and update time (`db::repository::agent_skills::validate_tools_json`) —
-- an unknown tool name is rejected immediately rather than silently
-- accepted and only discovered to do nothing once a run actually starts.
-- Defaults to the "all tools" sentinel so a skill created without
-- specifying `tools_json` behaves like an unrestricted agent today.
--
-- `preferred_model_id` is a plain reference to an existing `model_configs`
-- row (no new provider/model work here — see the M19 plan's own scope
-- note) — `ON DELETE SET NULL` so deleting a model config never blocks or
-- silently breaks a skill that preferred it; it just falls back to the
-- ordinary default model resolution.
CREATE TABLE agent_skills (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT,
  instructions TEXT NOT NULL,
  tools_json TEXT NOT NULL DEFAULT '["*"]',
  preferred_model_id TEXT REFERENCES model_configs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- Which skill (if any) an agent was created from. `ON DELETE SET NULL`:
-- deleting a skill must never cascade-delete the agents created from it —
-- it just stops being attributable to a (now gone) skill. The agent itself
-- already has its own copies of what matters (`system_prompt` was copied
-- from the skill's `instructions` at creation time — see `create_agent`),
-- so this column is purely "which skill, if any, produced this agent" for
-- display/tool-restriction/model-preference purposes at run time, not the
-- only place that data lives.
ALTER TABLE agents ADD COLUMN skill_id TEXT REFERENCES agent_skills(id) ON DELETE SET NULL;

CREATE INDEX idx_agents_skill_id ON agents(skill_id);

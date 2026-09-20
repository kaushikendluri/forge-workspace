-- M11: agent-to-agent structured messages within a mission. Written by the
-- `send_message` tool (`agent::tools`), available to an agent's M6 tool loop
-- only when that run is part of a mission (see
-- `agent::tool_loop::run_agent_loop_inner`'s conditional tool list). Read
-- back by `list_agent_messages` for Mission Control's messages panel and
-- `AgentDetail.tsx`'s activity stream — a persisted communication log, not a
-- live chat, so `to_agent_run_id` may point at a run that has already
-- finished (or hasn't started yet) by the time it's read.
CREATE TABLE agent_messages (
  id TEXT PRIMARY KEY,
  from_agent_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
  -- NULL = broadcast to the whole mission (no specific `to_task_title` was
  -- given, or resolving it found no run for that task yet).
  to_agent_run_id TEXT REFERENCES agent_runs(id) ON DELETE SET NULL,
  mission_id TEXT NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
  subject TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_agent_messages_mission_id ON agent_messages(mission_id);
CREATE INDEX idx_agent_messages_from_run ON agent_messages(from_agent_run_id);
CREATE INDEX idx_agent_messages_to_run ON agent_messages(to_agent_run_id);

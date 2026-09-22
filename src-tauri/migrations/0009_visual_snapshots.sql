-- M16: visual regression snapshots captured by the agent's real
-- `browser_screenshot` tool (see `agent::tools::browser_screenshot_tool`).
-- Screenshots are real PNG files saved under the app's own data directory
-- (never inlined as base64 blobs in SQLite, per the original Phase 4 spec)
-- and referenced here only by path. `kind` is decided at capture time: the
-- first screenshot for a given `(agent_run_id, label)` pair is the
-- 'baseline', every later one with the same label is a 'comparison' against
-- it — see `db::repository::visual_snapshots::has_baseline_for_label`.
-- `task_id` is set only when the run belongs to a mission task (NULL for a
-- solo run) — best-effort context for the "Ask to fix" follow-up action,
-- not a hard requirement. `flagged` is the honest, minimal state behind the
-- Visual Regression panel's "Reject" action ("accept_as_baseline"/
-- "set_flagged" in `db::repository::visual_snapshots`): rejecting a
-- comparison does not (and cannot) automatically revert anything, it just
-- marks it for a human's attention.
CREATE TABLE visual_snapshots (
  id TEXT PRIMARY KEY,
  agent_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
  task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  label TEXT NOT NULL,
  image_path TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('baseline', 'comparison')),
  flagged INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_visual_snapshots_agent_run_id ON visual_snapshots(agent_run_id, label, created_at);

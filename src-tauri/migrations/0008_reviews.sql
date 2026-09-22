-- M14: the reviewer agent's structured output — one row per review attempt
-- of a completed agent run (`agent::reviewer::run_review`). `score` is
-- 0-100 (100 = flawless); `status` is `'pending'` while a review is in
-- flight (a row is inserted before any model call is made, so
-- `Tasks.tsx`'s board can show a task genuinely sitting in Review), then
-- `'passed'`/`'failed'` once it completes, based on
-- `agent::reviewer::PASS_THRESHOLD`. A review that errors before completing
-- (no API key, an API error, a malformed response) never leaves a stuck
-- `'pending'` row behind — `run_review` deletes it on failure instead, so
-- `get_review` honestly reports "no review" rather than a fabricated one.
CREATE TABLE reviews (
  id TEXT PRIMARY KEY,
  agent_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
  score INTEGER NOT NULL,
  findings_json TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'passed', 'failed')) DEFAULT 'pending',
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_reviews_agent_run_id ON reviews(agent_run_id, created_at DESC);

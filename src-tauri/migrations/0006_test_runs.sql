-- M12: persisted history of manually-triggered test/lint/build runs (the
-- Testing tab's "Run tests"/"Run lint"/"Run build" buttons), independent of
-- any agent run — see `commands::testing_commands::run_test_suite`.
CREATE TABLE test_runs (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('test', 'lint', 'build')),
  command TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running', 'success', 'failure')) DEFAULT 'running',
  output TEXT,
  exit_code INTEGER,
  started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  completed_at TEXT
);

CREATE INDEX idx_test_runs_project_kind_started ON test_runs(project_id, kind, started_at DESC);

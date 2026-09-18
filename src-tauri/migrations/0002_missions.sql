-- M8: mission planning. A mission turns a plain-English objective into a
-- structured, human-approved task plan (no execution happens until a later
-- milestone consumes `status = 'approved'`).
CREATE TABLE missions (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  objective TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('planning','plan_ready','approved','running','completed','failed')) DEFAULT 'planning',
  plan_json TEXT,
  error_message TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  approved_at TEXT,
  completed_at TEXT
);

CREATE INDEX idx_missions_project_id ON missions(project_id);

-- Rebuild `tasks` to add the M8 mission-planning columns and extend its
-- `status` CHECK with 'backlog' (planned by a mission, not yet started).
-- SQLite has no `ALTER TABLE ... ALTER COLUMN` / `DROP CONSTRAINT`, so
-- widening an existing CHECK constraint requires the documented rebuild
-- procedure: create the new table, copy the old data across, drop the old
-- table, then rename the new one into place. `depends_on_task_id` is
-- declared as `REFERENCES tasks(...)` — the *final* name this table will
-- have once renamed below, not the temporary `tasks_new` — so the
-- self-reference is already correct the moment the rename happens, rather
-- than depending on SQLite to rewrite an internal self-reference during
-- `RENAME TO` (which it reliably does for *other* tables' FKs pointing at
-- the renamed table, but is a needless risk to lean on for a table's
-- reference to itself). SQLite resolves FK target names lazily (at
-- enforcement time, not at `CREATE TABLE` parse time), so referencing the
-- not-yet-existent-under-this-name `tasks` here is valid.
CREATE TABLE tasks_new (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  mission_id TEXT REFERENCES missions(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL CHECK (status IN ('backlog','todo','in_progress','done')) DEFAULT 'todo',
  priority TEXT NOT NULL CHECK (priority IN ('low','medium','high')) DEFAULT 'medium',
  position INTEGER NOT NULL DEFAULT 0,
  depends_on_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  agent_type TEXT,
  agent_run_id TEXT REFERENCES agent_runs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT INTO tasks_new (id, project_id, title, description, status, agent_run_id, created_at, updated_at)
SELECT id, project_id, title, description, status, agent_run_id, created_at, updated_at FROM tasks;

DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

CREATE INDEX idx_tasks_project_id ON tasks(project_id);
CREATE INDEX idx_tasks_mission_id ON tasks(mission_id);
CREATE INDEX idx_tasks_depends_on ON tasks(depends_on_task_id);

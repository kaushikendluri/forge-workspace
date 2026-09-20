-- M9: the scheduler executes an approved mission's task graph. Widens the
-- `missions.status` and `tasks.status` CHECK constraints to add the
-- terminal states the scheduler needs to represent honestly:
--   * missions.status: 'stopped' — the mission was cancelled mid-flight
--     (`stop_mission`), distinct from 'failed' (a real error/blocked task)
--     and 'completed' (every task succeeded).
--   * tasks.status: 'failed' (its own agent run didn't succeed), 'blocked'
--     (its dependency ended failed/blocked/cancelled, or it's part of a
--     dependency cycle — see `orchestrator::scheduler`), 'cancelled' (the
--     mission was stopped before this task got a chance to run).
--
-- SQLite has no `ALTER TABLE ... ALTER COLUMN` / `DROP CONSTRAINT`, so
-- widening a CHECK constraint requires the same rebuild procedure
-- `0002_missions.sql` already used: create the new table, copy the old
-- data across, drop the old table, rename the new one into place.
--
-- Note on `PRAGMA foreign_keys`: `rusqlite_migration` runs every migration's
-- SQL inside one transaction, and SQLite treats `PRAGMA foreign_keys` as a
-- no-op when issued inside a transaction — so despite `0001_init.sql`
-- setting it to `ON`, foreign key enforcement has never actually been
-- active for this app's connections (confirmed empirically; out of scope to
-- change here). That means dropping `missions`/`tasks` below, even while
-- rows in other tables reference them, is safe today regardless of rebuild
-- order — but the rebuild is still written parent-before-child (`missions`
-- before `tasks`, which references it) so it stays correct if that ever
-- changes.

CREATE TABLE missions_new (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  objective TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('planning','plan_ready','approved','running','completed','failed','stopped')) DEFAULT 'planning',
  plan_json TEXT,
  error_message TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  approved_at TEXT,
  completed_at TEXT
);

INSERT INTO missions_new (id, project_id, objective, status, plan_json, error_message, created_at, approved_at, completed_at)
SELECT id, project_id, objective, status, plan_json, error_message, created_at, approved_at, completed_at FROM missions;

DROP TABLE missions;
ALTER TABLE missions_new RENAME TO missions;

CREATE INDEX idx_missions_project_id ON missions(project_id);

CREATE TABLE tasks_new (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  mission_id TEXT REFERENCES missions(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL CHECK (status IN ('backlog','todo','in_progress','done','failed','blocked','cancelled')) DEFAULT 'todo',
  priority TEXT NOT NULL CHECK (priority IN ('low','medium','high')) DEFAULT 'medium',
  position INTEGER NOT NULL DEFAULT 0,
  depends_on_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  agent_type TEXT,
  agent_run_id TEXT REFERENCES agent_runs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT INTO tasks_new (id, project_id, mission_id, title, description, status, priority, position, depends_on_task_id, agent_type, agent_run_id, created_at, updated_at)
SELECT id, project_id, mission_id, title, description, status, priority, position, depends_on_task_id, agent_type, agent_run_id, created_at, updated_at FROM tasks;

DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

CREATE INDEX idx_tasks_project_id ON tasks(project_id);
CREATE INDEX idx_tasks_mission_id ON tasks(mission_id);
CREATE INDEX idx_tasks_depends_on ON tasks(depends_on_task_id);

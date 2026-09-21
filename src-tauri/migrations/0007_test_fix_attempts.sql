-- M13: tracks the self-healing test-fix cycle's bounded retry counter per
-- run — see `agent::test_fix::TestFixTracker`, which counts a "fix attempt"
-- for every `run_tests` tool call whose immediately preceding `run_tests`
-- call came back as a failure. Mirrored here so the run's history/UI can see
-- it without replaying its whole activity feed.
ALTER TABLE agent_runs ADD COLUMN test_fix_attempts INTEGER NOT NULL DEFAULT 0;

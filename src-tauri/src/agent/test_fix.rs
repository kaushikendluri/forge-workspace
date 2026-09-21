//! M13: tracks the "self-healing" test-fix cycle within one agent run.
//!
//! M6's tool loop (`agent::tool_loop`) already, naturally, lets a model see
//! a `run_tests` failure as a normal `tool_result` and try again on its own
//! — that's just the general loop. This module's job is narrower: turn that
//! implicit behavior into something explicit, bounded, and visible —
//! counting how many times the model has retried tests after a failure,
//! capping that count with its own budget (separate from the run's general
//! `max_iterations`), and telling `agent::tool_loop` exactly what happened
//! so it can emit a `test_fix_cycle` activity event a user can actually
//! read.
//!
//! # Heuristic
//! A "fix attempt" is counted for every `run_tests` call whose *immediately
//! preceding* `run_tests` call (if any) came back as a failure — regardless
//! of what happened in between (`write_file`/`edit_file`/anything else;
//! `agent::tool_loop` only ever calls [`TestFixTracker::observe`] for actual
//! `run_tests` results, so intervening tool calls are already irrelevant by
//! construction). This deliberately does not try to verify the model
//! actually edited a file in between: "the agent saw a failure and ran the
//! tests again" is already strong, simple evidence of a fix attempt,
//! whether or not that attempt succeeds.
//!
//! A chain of `N` consecutive `run_tests` failures followed by one passing
//! run therefore counts as `N` fix attempts (one per consecutive pair) —
//! see the `a_chain_of_failures_counts_one_attempt_per_consecutive_pair`
//! test below.

/// One thing worth surfacing in the activity stream, produced by
/// [`TestFixTracker::observe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestFixEvent {
    /// A fresh `run_tests` failure with no fix attempt already in progress.
    /// `attempt` is the number this failure's *eventual* retry would become
    /// if the model tries again.
    Failed { attempt: i64 },
    /// A `run_tests` call that completes a pending fix attempt (whether or
    /// not that same call is itself also a failure — see the module docs).
    Retested { attempt: i64, passed: bool },
}

/// Per-run state. Lives on `agent::tool_loop::run_agent_loop_inner`'s stack;
/// its `attempts()` is mirrored onto the `agent_runs.test_fix_attempts`
/// column (via `db::repository::agent_runs::set_test_fix_attempts`)
/// whenever it changes, rather than being the column's source of truth
/// itself — a single run only ever has one tracker, updated sequentially,
/// so there's no concurrent-write concern to design around.
#[derive(Debug, Default)]
pub struct TestFixTracker {
    last_failed: bool,
    attempts: i64,
    budget_exhausted: bool,
}

impl TestFixTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attempts(&self) -> i64 {
        self.attempts
    }

    /// Call once per `run_tests` tool result, with whether that call came
    /// back as a tool error. Returns, in order, the events this call
    /// produced — at most one: a single `run_tests` call is either a fresh
    /// failure or a retest of a pending one, never both.
    pub fn observe(&mut self, is_error: bool) -> Vec<TestFixEvent> {
        let mut events = Vec::new();
        let prior_failed = self.last_failed;

        if prior_failed {
            self.attempts += 1;
            events.push(TestFixEvent::Retested { attempt: self.attempts, passed: !is_error });
        }

        if is_error {
            if !prior_failed {
                events.push(TestFixEvent::Failed { attempt: self.attempts + 1 });
            }
            self.last_failed = true;
        } else {
            self.last_failed = false;
        }

        events
    }

    /// `Some(attempts)` exactly once per run — the first call after
    /// `attempts` exceeds `cap`. Every later call returns `None` even as
    /// `attempts` keeps growing, so the caller's "budget exhausted" activity
    /// event and terminal stop only ever fire once.
    pub fn check_budget(&mut self, cap: i64) -> Option<i64> {
        if !self.budget_exhausted && self.attempts > cap {
            self.budget_exhausted = true;
            Some(self.attempts)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_events_for_a_passing_run_tests_with_no_prior_failure() {
        let mut t = TestFixTracker::new();
        assert_eq!(t.observe(false), vec![]);
        assert_eq!(t.attempts(), 0);
    }

    #[test]
    fn first_failure_emits_failed_with_attempt_one_but_does_not_count_yet() {
        let mut t = TestFixTracker::new();
        assert_eq!(t.observe(true), vec![TestFixEvent::Failed { attempt: 1 }]);
        assert_eq!(t.attempts(), 0, "an attempt only counts once it's actually retried");
    }

    #[test]
    fn a_retest_after_failure_counts_as_attempt_one() {
        let mut t = TestFixTracker::new();
        t.observe(true); // fails
        let events = t.observe(false); // retest, passes
        assert_eq!(events, vec![TestFixEvent::Retested { attempt: 1, passed: true }]);
        assert_eq!(t.attempts(), 1);
    }

    #[test]
    fn a_chain_of_failures_counts_one_attempt_per_consecutive_pair() {
        let mut t = TestFixTracker::new();
        assert_eq!(t.observe(true), vec![TestFixEvent::Failed { attempt: 1 }]); // fail 1
        assert_eq!(t.observe(true), vec![TestFixEvent::Retested { attempt: 1, passed: false }]); // fail 2 -> retest of attempt 1
        assert_eq!(t.observe(true), vec![TestFixEvent::Retested { attempt: 2, passed: false }]); // fail 3 -> retest of attempt 2
        assert_eq!(t.observe(false), vec![TestFixEvent::Retested { attempt: 3, passed: true }]); // pass -> retest of attempt 3
        assert_eq!(t.attempts(), 3);
    }

    #[test]
    fn a_pass_after_a_pass_produces_no_events_and_does_not_count() {
        let mut t = TestFixTracker::new();
        t.observe(false);
        assert_eq!(t.observe(false), vec![]);
        assert_eq!(t.attempts(), 0);
    }

    #[test]
    fn budget_check_fires_exactly_once_when_attempts_exceeds_cap() {
        let mut t = TestFixTracker::new();
        let cap = 3;
        // One failure plus 3 consecutive failing retests == attempts reaching
        // exactly the cap (still within budget).
        for _ in 0..4 {
            t.observe(true);
        }
        assert_eq!(t.attempts(), 3);
        assert_eq!(t.check_budget(cap), None);

        // A 5th consecutive failure pushes attempts to 4, exceeding the cap.
        t.observe(true);
        assert_eq!(t.attempts(), 4);
        assert_eq!(t.check_budget(cap), Some(4));

        // Never fires again, even though attempts keeps growing.
        t.observe(true);
        assert_eq!(t.attempts(), 5);
        assert_eq!(t.check_budget(cap), None);
    }

    /// The termination proof the M13 spec asks for: a pathological "always
    /// fails, model always tries again" sequence of `run_tests` calls trips
    /// the budget at `cap + 1` attempts — a small, fixed number — long
    /// before anywhere near a general loop's `max_iterations` (40 by
    /// default in `agent::tool_loop`, but this asserts against a much
    /// larger bound so the proof doesn't depend on that default). This is
    /// what actually bounds this specific pathological pattern — not
    /// `max_iterations`, by coincidence or otherwise.
    #[test]
    fn pathological_always_failing_sequence_trips_the_budget_at_the_cap_not_at_max_iterations() {
        let mut t = TestFixTracker::new();
        let cap = 3;
        let max_iterations_far_above_any_sane_cap = 10_000;

        let mut tripped_at: Option<(usize, i64)> = None;
        for call in 0..max_iterations_far_above_any_sane_cap {
            t.observe(true); // always fails, and the model always retries
            if let Some(attempts) = t.check_budget(cap) {
                tripped_at = Some((call, attempts));
                break;
            }
        }

        let (call, attempts) = tripped_at.expect("the budget must trip well before max_iterations");
        assert_eq!(attempts, cap + 1, "budget trips exactly one attempt past the cap");
        assert!(call < 10, "must trip almost immediately (at the cap), not anywhere near max_iterations; tripped at call {call}");
    }
}

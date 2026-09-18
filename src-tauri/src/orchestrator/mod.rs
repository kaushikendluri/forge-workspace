//! Phase 3 mission orchestration: turning a plain-English objective into a
//! structured, human-approved task plan (M8). Deliberately separate from
//! `agent::tool_loop` — a mission's plan comes from one single-turn,
//! forced-tool-use Anthropic call (`planner::propose_plan`), never the
//! multi-turn tool-calling agent loop that executes a task once approved.
//! Nothing in this module (or the rest of M8) executes a plan — that's a
//! later milestone (M9) consuming `missions.status = 'approved'`.

pub mod planner;

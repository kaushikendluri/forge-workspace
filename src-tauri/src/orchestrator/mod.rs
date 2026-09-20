//! Phase 3 mission orchestration: turning a plain-English objective into a
//! structured, human-approved task plan (M8). Deliberately separate from
//! `agent::tool_loop` — a mission's plan comes from one single-turn,
//! forced-tool-use Anthropic call (`planner::propose_plan`), never the
//! multi-turn tool-calling agent loop that executes a task once approved.
//! M8's `planner` only *proposes* a plan; M9's `scheduler` is what actually
//! consumes `missions.status = 'approved'` and executes it, sequentially,
//! through the existing M5/M6 agent pipeline.

pub mod events;
pub mod planner;
pub mod scheduler;

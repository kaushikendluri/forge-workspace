//! One module per table, each holding the CRUD functions `commands/*.rs`
//! will call. Bodies are `todo!()` stubs in M1 — real queries land
//! alongside the commands that need them (M2+).

pub mod activity_events;
pub mod agent_runs;
pub mod agents;
pub mod model_configs;
pub mod notifications;
pub mod projects;
pub mod repositories;
pub mod settings;
pub mod tasks;
pub mod tool_calls;
pub mod workspaces;

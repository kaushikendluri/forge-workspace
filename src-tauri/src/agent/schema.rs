//! JSON Schema tool definitions for the agent-facing tool set, in
//! Anthropic's `tools` array format (`name`/`description`/`input_schema`).
//! Dispatch for each of these lives in `agent::tools`.

use serde_json::json;

use super::anthropic_client::ToolDefinition;

fn tool(name: &str, description: &str, input_schema: serde_json::Value) -> ToolDefinition {
    ToolDefinition { name: name.to_string(), description: description.to_string(), input_schema }
}

/// Every tool the agent loop may call, in a stable order (stable order
/// matters for prompt caching — see `agent::tool_loop`).
pub fn all_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        tool(
            "read_file",
            "Read the full contents of a text file inside the workspace. Path must be relative to the \
             workspace root.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Path relative to the workspace root." } },
                "required": ["path"],
            }),
        ),
        tool(
            "write_file",
            "Create a new file or overwrite an existing file's entire contents. Path must be relative to \
             the workspace root; missing parent directories are created automatically.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the workspace root." },
                    "content": { "type": "string", "description": "The file's full new contents." },
                },
                "required": ["path", "content"],
            }),
        ),
        tool(
            "edit_file",
            "Replace one exact occurrence of `old_string` with `new_string` in an existing file. Fails \
             clearly if `old_string` is not found, or is found more than once (make it longer/more specific \
             to disambiguate) — never guesses which occurrence you meant.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the workspace root." },
                    "old_string": { "type": "string", "description": "Exact text to replace; must be unique in the file." },
                    "new_string": { "type": "string", "description": "Replacement text." },
                },
                "required": ["path", "old_string", "new_string"],
            }),
        ),
        tool(
            "list_directory",
            "List the immediate contents (files and subdirectories) of a directory inside the workspace.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Path relative to the workspace root ('.' for the workspace root itself)." } },
                "required": ["path"],
            }),
        ),
        tool(
            "search_files",
            "Find files by name using a glob-style pattern (e.g. '*.ts', 'src/**/*.rs'), searched from the \
             workspace root.",
            json!({
                "type": "object",
                "properties": { "pattern": { "type": "string", "description": "Glob pattern, e.g. '**/*.tsx'." } },
                "required": ["pattern"],
            }),
        ),
        tool(
            "search_code",
            "Search file contents for a query string across the workspace. Plain-text substring search by \
             default; set `regex` to true to treat `query` as a regular expression.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "regex": { "type": "boolean", "description": "Treat `query` as a regular expression. Defaults to false." },
                },
                "required": ["query"],
            }),
        ),
        tool(
            "run_command",
            "Run a shell command in the workspace root and capture its stdout/stderr/exit code. Runs with a \
             timeout; a command that hangs is killed rather than blocking the run forever.",
            json!({
                "type": "object",
                "properties": { "command": { "type": "string", "description": "The full shell command to run." } },
                "required": ["command"],
            }),
        ),
        tool(
            "run_tests",
            "Run the project's configured test command. Returns a clear error result (not a guessed command) \
             if no test command has been configured for this project.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "run_linter",
            "Run the project's configured lint command. Returns a clear error result (not a guessed command) \
             if no lint command has been configured for this project.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "run_build",
            "Run the project's configured build command. Returns a clear error result (not a guessed command) \
             if no build command has been configured for this project.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "git_status",
            "Show the workspace's current git status: current branch and staged/unstaged/untracked files.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "git_diff",
            "Show the before/after content of one changed file in the workspace, relative to HEAD. Call \
             `git_status` first to find changed file paths.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Path (relative to the workspace root) of the changed file to diff." } },
                "required": ["path"],
            }),
        ),
        tool(
            "git_log",
            "Show the most recent commits in the workspace, most recent first.",
            json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "description": "Maximum number of commits to return. Defaults to 20." } },
            }),
        ),
        tool(
            "report_completion",
            "Call this exactly once, when the task is fully finished (or when it cannot be completed), to \
             end the run. `success` should be false if the task could not be completed.",
            json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "A clear summary of what was done (or why the task could not be completed)." },
                    "success": { "type": "boolean" },
                },
                "required": ["summary", "success"],
            }),
        ),
    ]
}

/// M11: agent-to-agent structured messaging. Deliberately **not** included
/// in [`all_tool_definitions`] — it's only meaningful when a run is
/// executing as part of a mission, so `agent::tool_loop::run_agent_loop_inner`
/// appends this to the model's tool list itself, only for a run whose
/// `ToolContext::mission_context` is `Some`. A solo M6 run (started directly
/// from the Agents page) never sees it offered at all. `agent::tools::
/// dispatch_tool`'s `"send_message"` branch is still defensive about being
/// called without mission context anyway (a clear tool-result error, not a
/// crash), in case that ever changes.
pub fn send_message_tool_definition() -> ToolDefinition {
    tool(
        "send_message",
        "Send a structured message to another task's agent in this mission, or broadcast to the whole \
         mission by omitting `to_task_title`. This is a persisted mission-level communication log, not a \
         live chat — the recipient task may not have started yet, or may have already finished, and the \
         message is still recorded either way. Only available when this run is executing as part of a \
         mission.",
        json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "A short subject line for the message." },
                "body": { "type": "string", "description": "The message body." },
                "to_task_title": {
                    "type": "string",
                    "description": "The exact title of another task in this mission to address the message to. Omit to broadcast to every task in the mission.",
                },
            },
            "required": ["subject", "body"],
        }),
    )
}

/// M14: every mutating tool a reviewer run must never be offered — kept as
/// its own list (rather than defined only implicitly by
/// [`REVIEWER_TOOL_NAMES`]'s absence) so the exclusion is asserted directly
/// in `reviewer_tool_list_excludes_every_mutating_tool` below, not just
/// implied by what a maintainer remembered to leave out of an allowlist.
const MUTATING_TOOL_NAMES: [&str; 7] =
    ["write_file", "edit_file", "run_command", "run_tests", "run_linter", "run_build", "report_completion"];

/// M14: the read-only subset of [`all_tool_definitions`] offered to a
/// reviewer run (`agent::reviewer`) — every tool that only *reads* the
/// workspace (files, directory listings, code search, git history/diff/
/// status), none that can mutate it or run arbitrary shell commands, and
/// not `report_completion` either (a reviewer ends its context-gathering
/// phase simply by not calling another tool — see `agent::reviewer`'s own
/// docs). Filters the exact same [`ToolDefinition`]s `all_tool_definitions`
/// builds, rather than a hand-maintained separate list, so a tool's schema
/// can never drift between a normal run and a reviewer run.
const REVIEWER_TOOL_NAMES: [&str; 6] = ["read_file", "list_directory", "search_code", "git_diff", "git_status", "git_log"];

pub fn reviewer_tool_definitions() -> Vec<ToolDefinition> {
    all_tool_definitions().into_iter().filter(|d| REVIEWER_TOOL_NAMES.contains(&d.name.as_str())).collect()
}

/// M15: the tool list offered to the bounded AI conflict resolver
/// (`agent::conflict_resolver`) — narrower even than the reviewer's
/// read-only set, but for a different reason: this one *can* mutate files
/// (it has to, to actually resolve conflict markers), so it's scoped instead
/// to exactly the tools that job needs and nothing more. `read_file`/
/// `edit_file` to see and fix conflict markers (further restricted, outside
/// this list, to only the files git itself flagged as conflicted — see
/// `agent::conflict_resolver`), `git_status`/`git_diff` for orientation. No
/// `write_file` (it must only edit files git already flagged, never create
/// new ones), no `run_command`/`run_tests`/`run_linter`/`run_build` (its job
/// is narrowly resolving conflict markers, not doing general work), and no
/// `report_completion` (same "just stop calling tools" pattern the
/// reviewer's context-gathering phase already uses).
const CONFLICT_RESOLVER_TOOL_NAMES: [&str; 4] = ["read_file", "edit_file", "git_status", "git_diff"];

pub fn conflict_resolver_tool_definitions() -> Vec<ToolDefinition> {
    all_tool_definitions().into_iter().filter(|d| CONFLICT_RESOLVER_TOOL_NAMES.contains(&d.name.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_unique_name_and_object_schema() {
        let defs = all_tool_definitions();
        let mut names = std::collections::HashSet::new();
        for def in &defs {
            assert!(names.insert(def.name.clone()), "duplicate tool name: {}", def.name);
            assert_eq!(def.input_schema.get("type").and_then(|v| v.as_str()), Some("object"));
            assert!(!def.description.is_empty());
        }
        assert!(names.contains("report_completion"));
        assert!(names.contains("run_command"));
        assert!(
            !names.contains("send_message"),
            "send_message must not be unconditionally offered — only mission-context runs should see it \
             (agent::tool_loop appends it itself)"
        );
    }

    #[test]
    fn send_message_tool_definition_is_well_formed_and_distinct() {
        let def = send_message_tool_definition();
        assert_eq!(def.name, "send_message");
        assert_eq!(def.input_schema.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(!def.description.is_empty());
        assert!(!all_tool_definitions().iter().any(|d| d.name == def.name));
    }

    /// M14's core safety property, verified by construction: a reviewer run
    /// must never be offered a single mutating tool. Checks the actual
    /// returned list against every name in [`MUTATING_TOOL_NAMES`] (plus
    /// `send_message`, which is mission-messaging, not read-only either)
    /// rather than merely trusting [`REVIEWER_TOOL_NAMES`]'s own contents.
    #[test]
    fn reviewer_tool_list_excludes_every_mutating_tool() {
        let reviewer_tools = reviewer_tool_definitions();
        let reviewer_names: std::collections::HashSet<&str> = reviewer_tools.iter().map(|d| d.name.as_str()).collect();

        for mutating in MUTATING_TOOL_NAMES {
            assert!(!reviewer_names.contains(mutating), "reviewer tool list must not include '{mutating}'");
        }
        assert!(!reviewer_names.contains("send_message"), "reviewer tool list must not include send_message either");
    }

    #[test]
    fn reviewer_tool_list_is_exactly_the_read_only_tools() {
        let reviewer_tools = reviewer_tool_definitions();
        let mut names: Vec<&str> = reviewer_tools.iter().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        let mut expected: Vec<&str> = REVIEWER_TOOL_NAMES.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected);
        // Every one of these must also be a real, defined tool (not a typo
        // that would silently offer nothing) — reuses the exact same
        // `ToolDefinition`s `all_tool_definitions` builds.
        for def in &reviewer_tools {
            assert_eq!(def.input_schema.get("type").and_then(|v| v.as_str()), Some("object"));
            assert!(!def.description.is_empty());
        }
    }

    /// M15's core safety property for the conflict resolver's tool list,
    /// mirroring `reviewer_tool_list_excludes_every_mutating_tool`: it must
    /// never be offered `write_file` (only `edit_file`, on files it's
    /// further scoped to outside this list) or any shell/completion tool.
    #[test]
    fn conflict_resolver_tool_list_excludes_write_file_and_every_shell_and_completion_tool() {
        let tools = conflict_resolver_tool_definitions();
        let names: std::collections::HashSet<&str> = tools.iter().map(|d| d.name.as_str()).collect();

        for excluded in ["write_file", "run_command", "run_tests", "run_linter", "run_build", "report_completion", "send_message"] {
            assert!(!names.contains(excluded), "conflict resolver tool list must not include '{excluded}'");
        }
        assert!(names.contains("read_file"));
        assert!(names.contains("edit_file"));
        assert!(names.contains("git_status"));
        assert!(names.contains("git_diff"));
        assert_eq!(names.len(), 4);
    }
}

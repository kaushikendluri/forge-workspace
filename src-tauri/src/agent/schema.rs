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
}

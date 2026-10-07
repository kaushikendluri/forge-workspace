//! CRUD for the `agent_memory` table (Phase 5 M18). See
//! `migrations/0011_agent_memory.sql` for the table's shape and scoping
//! reasoning, and `agent::memory` for the (pure, DB-free) extraction and
//! retrieval-ranking logic this repository's rows feed into.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::models::{AgentMemory, AgentMemoryKind};
use crate::error::AppResult;

fn kind_str(kind: AgentMemoryKind) -> &'static str {
    match kind {
        AgentMemoryKind::Decision => "decision",
        AgentMemoryKind::FileContext => "file_context",
        AgentMemoryKind::Error => "error",
        AgentMemoryKind::CompletedWork => "completed_work",
    }
}

fn parse_kind(s: &str) -> AgentMemoryKind {
    match s {
        "file_context" => AgentMemoryKind::FileContext,
        "error" => AgentMemoryKind::Error,
        "completed_work" => AgentMemoryKind::CompletedWork,
        _ => AgentMemoryKind::Decision,
    }
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentMemory> {
    let kind: String = row.get(3)?;
    Ok(AgentMemory {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        agent_run_id: row.get(2)?,
        kind: parse_kind(&kind),
        content: row.get(4)?,
        relevance_tags: row.get(5)?,
        created_at: row.get(6)?,
    })
}

const SELECT_COLUMNS: &str = "id, agent_id, agent_run_id, kind, content, relevance_tags, created_at";

/// All of `agent_id`'s memory, most recently created first. Deliberately
/// unfiltered — `agent::memory::rank_relevant_memories` is what narrows
/// this down to a small, genuinely relevant subset before it's ever shown
/// to a model or the frontend; this function alone is never meant to back
/// a "dump everything into the prompt" path.
pub fn list_for_agent(conn: &Connection, agent_id: &str) -> AppResult<Vec<AgentMemory>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {SELECT_COLUMNS} FROM agent_memory WHERE agent_id = ?1 ORDER BY created_at DESC"))?;
    let rows = stmt.query_map(params![agent_id], row_to_memory)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_by_id(conn: &Connection, id: &str) -> AppResult<Option<AgentMemory>> {
    conn.query_row(&format!("SELECT {SELECT_COLUMNS} FROM agent_memory WHERE id = ?1"), params![id], row_to_memory)
        .optional()
        .map_err(Into::into)
}

/// Inserts one memory row — called by `agent::tool_loop::finish_run` once
/// per entry `agent::memory::extract_memories_from_run` produced for the
/// run that just ended. `agent_run_id` is `None` only in tests; a real run
/// always has one.
pub fn insert(
    conn: &Connection,
    agent_id: &str,
    agent_run_id: Option<&str>,
    kind: AgentMemoryKind,
    content: &str,
    relevance_tags: &str,
) -> AppResult<AgentMemory> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_memory (id, agent_id, agent_run_id, kind, content, relevance_tags, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, agent_id, agent_run_id, kind_str(kind), content, relevance_tags, now],
    )?;
    Ok(AgentMemory {
        id,
        agent_id: agent_id.to_string(),
        agent_run_id: agent_run_id.map(str::to_string),
        kind,
        content: content.to_string(),
        relevance_tags: relevance_tags.to_string(),
        created_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn.execute("INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')", []).expect("insert project");
        conn.execute("INSERT INTO repositories (id, project_id, root_path) VALUES ('r1', 'p1', '/tmp/r1')", [])
            .expect("insert repository");
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a1', 'p1', 'r1', 'Bot')", [])
            .expect("insert agent");
        conn.execute(
            "INSERT INTO agent_runs (id, agent_id, task_prompt, model_id) VALUES ('run1', 'a1', 'do it', 'claude-sonnet-5')",
            [],
        )
        .expect("insert run");
        conn
    }

    #[test]
    fn insert_then_get_round_trips() {
        let conn = setup_conn();
        let memory = insert(&conn, "a1", Some("run1"), AgentMemoryKind::CompletedWork, "Fixed the login bug", "login bug fix")
            .expect("insert");
        assert_eq!(memory.kind, AgentMemoryKind::CompletedWork);

        let fetched = get_by_id(&conn, &memory.id).expect("get_by_id").expect("exists");
        assert_eq!(fetched.content, "Fixed the login bug");
        assert_eq!(fetched.relevance_tags, "login bug fix");
        assert_eq!(fetched.agent_run_id.as_deref(), Some("run1"));
    }

    #[test]
    fn insert_with_no_run_id_round_trips_as_none() {
        let conn = setup_conn();
        let memory = insert(&conn, "a1", None, AgentMemoryKind::Decision, "Chose SQLite over Postgres", "sqlite postgres")
            .expect("insert");
        assert!(memory.agent_run_id.is_none());
        let fetched = get_by_id(&conn, &memory.id).expect("get_by_id").expect("exists");
        assert!(fetched.agent_run_id.is_none());
    }

    #[test]
    fn list_for_agent_returns_all_of_its_memory() {
        let conn = setup_conn();
        insert(&conn, "a1", Some("run1"), AgentMemoryKind::FileContext, "Touched src/a.rs", "src a.rs").expect("insert 1");
        insert(&conn, "a1", Some("run1"), AgentMemoryKind::Error, "Build failed: missing semicolon", "build failed").expect("insert 2");

        let listed = list_for_agent(&conn, "a1").expect("list_for_agent");
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn list_for_agent_is_empty_for_an_agent_with_no_memory_yet() {
        let conn = setup_conn();
        assert!(list_for_agent(&conn, "a1").expect("list_for_agent").is_empty());
    }

    #[test]
    fn list_for_agent_never_returns_another_agents_memory() {
        let conn = setup_conn();
        conn.execute("INSERT INTO agents (id, project_id, repository_id, name) VALUES ('a2', 'p1', 'r1', 'Other Bot')", [])
            .expect("insert second agent");
        insert(&conn, "a1", None, AgentMemoryKind::Decision, "a1's own decision", "a1 decision").expect("insert for a1");
        insert(&conn, "a2", None, AgentMemoryKind::Decision, "a2's own decision", "a2 decision").expect("insert for a2");

        let listed = list_for_agent(&conn, "a1").expect("list_for_agent");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].content, "a1's own decision");
    }
}

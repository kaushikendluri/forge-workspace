//! Phase 5 M20: the single place implementing the model-selection precedence
//! every AI-calling call site in this app now follows, from most to least
//! specific:
//!
//!   1. an explicit per-call override (a real `model_configs.id`), when the
//!      caller has one — no current call site threads a genuine one through
//!      yet, but the parameter exists so a future "run this one call on
//!      model X" override can be added without touching this function
//!      again or re-deriving the precedence rule at its call site;
//!   2. an Agent Skill's `preferred_model_id` (M19's own field, unchanged),
//!      when the caller is resolving a model for an agent created from a
//!      skill (`commands::agent_commands::resolve_model_id_for_agent`);
//!   3. this call's [`ModelRole`]'s own configured default — the
//!      `model.role.<role>` setting (M20, see [`ModelRole::setting_key`]),
//!      editable from Settings;
//!   4. the single global default `model_configs` row
//!      (`model_configs.is_default = 1`);
//!   5. a last-resort hardcoded Anthropic Sonnet fallback, for the
//!      pathological case where `model_configs` itself has no default row
//!      at all (shouldn't happen — the M1 migration seeds one — but every
//!      prior milestone's own ad-hoc fallback already assumed a value this
//!      honest here, so this function keeps that promise in one place
//!      rather than letting each caller reinvent its own).
//!
//! Every step after (1)/(2) is a real DB lookup; an id that no longer
//! resolves to a row (e.g. a deleted model config) is treated exactly like
//! "not set" and falls through to the next tier, the same way M19's own
//! `preferred_model_id` handling already did — never an error.

use rusqlite::Connection;

use crate::db::models::ModelConfig;
use crate::db::repository::{model_configs as model_configs_repo, settings as settings_repo};
use crate::error::AppResult;

/// Which of this app's four coarse AI-calling responsibilities a model is
/// being resolved for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    /// M8's mission planner (`orchestrator::planner::propose_plan`) — turning
    /// a plain-English objective into a structured task plan.
    Orchestrator,
    /// M6's normal agent run (`agent::tool_loop`) — the hands-on coding loop
    /// every agent run drives by default (a skill's `preferred_model_id`
    /// still wins over this, per the precedence above).
    Coder,
    /// M14's reviewer (`agent::reviewer::run_review`).
    Reviewer,
    /// Cheap/bounded utility calls that aren't hands-on coding or planning.
    /// M17's Project Brain analysis (`brain::regenerate_brain`) is assigned
    /// here rather than `Orchestrator`: it's a single structured-output call
    /// over a shallow repo summary, not an interactive planning
    /// conversation, and keeping it off `Orchestrator`/`Coder` means a user
    /// can point those roles at a more expensive model without also paying
    /// that price for every background brain refresh
    /// (`maybe_auto_regenerate` runs on every project open).
    Utility,
}

impl ModelRole {
    /// The `settings` table key (M4's key/value pattern) this role's default
    /// model id (a `model_configs.id`) is stored under, if the user has set
    /// one from Settings. Unset (the default on any fresh/existing install)
    /// falls through to the global default — nothing breaks for an install
    /// that predates this milestone.
    pub fn setting_key(self) -> &'static str {
        match self {
            ModelRole::Orchestrator => "model.role.orchestrator",
            ModelRole::Coder => "model.role.coder",
            ModelRole::Reviewer => "model.role.reviewer",
            ModelRole::Utility => "model.role.utility",
        }
    }
}

const FALLBACK_MODEL_ID: &str = "claude-sonnet-5";
const FALLBACK_PROVIDER: &str = "anthropic";
const FALLBACK_MAX_OUTPUT_TOKENS: i64 = 8192;

/// The step-5 last-resort fallback — see this module's own docs for why it
/// exists and when it's actually reachable (effectively never, outside a
/// test that deliberately empties `model_configs`).
fn fallback_model_config() -> ModelConfig {
    ModelConfig {
        id: "fallback-anthropic-sonnet".to_string(),
        provider: FALLBACK_PROVIDER.to_string(),
        model_id: FALLBACK_MODEL_ID.to_string(),
        display_name: "Claude Sonnet 5 (built-in fallback)".to_string(),
        is_default: false,
        max_output_tokens: FALLBACK_MAX_OUTPUT_TOKENS,
        created_at: String::new(),
    }
}

/// Resolves the real `model_configs` row to use for one call, per this
/// module's own precedence order (see module docs). `explicit_override_id`/
/// `skill_preferred_model_id` are both plain `model_configs.id` references;
/// pass `None` for either when the caller has no such override to offer.
pub fn resolve_model_config(
    conn: &Connection,
    role: ModelRole,
    explicit_override_id: Option<&str>,
    skill_preferred_model_id: Option<&str>,
) -> AppResult<ModelConfig> {
    if let Some(id) = explicit_override_id {
        if let Some(model) = model_configs_repo::get_by_id(conn, id)? {
            return Ok(model);
        }
    }
    if let Some(id) = skill_preferred_model_id {
        if let Some(model) = model_configs_repo::get_by_id(conn, id)? {
            return Ok(model);
        }
    }
    if let Some(role_default_id) = settings_repo::get(conn, role.setting_key())?.map(|s| s.value) {
        if let Some(model) = model_configs_repo::get_by_id(conn, &role_default_id)? {
            return Ok(model);
        }
    }
    if let Some(model) = model_configs_repo::get_default(conn)? {
        return Ok(model);
    }
    Ok(fallback_model_config())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn migrated_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&mut conn).expect("run migrations");
        conn
    }

    fn insert_model(conn: &Connection, id: &str, provider: &str, model_id: &str, is_default: bool) {
        conn.execute(
            "INSERT INTO model_configs (id, provider, model_id, display_name, is_default, max_output_tokens) \
             VALUES (?1, ?2, ?3, ?3, ?4, 8192)",
            rusqlite::params![id, provider, model_id, is_default as i64],
        )
        .expect("insert model_configs row");
    }

    /// A fresh/existing install with no role settings and no explicit/skill
    /// override must resolve to exactly the M1-seeded global default — the
    /// "nothing breaks" guarantee this milestone promises.
    #[test]
    fn falls_back_to_the_global_default_when_nothing_more_specific_is_set() {
        let conn = migrated_conn();
        let resolved = resolve_model_config(&conn, ModelRole::Coder, None, None).expect("resolve");
        assert_eq!(resolved.id, "default-sonnet");
        assert_eq!(resolved.model_id, "claude-sonnet-5");
    }

    #[test]
    fn a_configured_role_default_wins_over_the_global_default() {
        let conn = migrated_conn();
        insert_model(&conn, "role-model", "openai", "gpt-4.1", false);
        settings_repo::set(&conn, ModelRole::Coder.setting_key(), "role-model").expect("set role default");

        let resolved = resolve_model_config(&conn, ModelRole::Coder, None, None).expect("resolve");
        assert_eq!(resolved.id, "role-model");
        assert_eq!(resolved.provider, "openai");

        // A different role with no configured default is unaffected.
        let reviewer_resolved = resolve_model_config(&conn, ModelRole::Reviewer, None, None).expect("resolve");
        assert_eq!(reviewer_resolved.id, "default-sonnet");
    }

    #[test]
    fn a_skill_preference_wins_over_a_configured_role_default() {
        let conn = migrated_conn();
        insert_model(&conn, "role-model", "openai", "gpt-4.1", false);
        insert_model(&conn, "skill-model", "google", "gemini-2.5-pro", false);
        settings_repo::set(&conn, ModelRole::Coder.setting_key(), "role-model").expect("set role default");

        let resolved = resolve_model_config(&conn, ModelRole::Coder, None, Some("skill-model")).expect("resolve");
        assert_eq!(resolved.id, "skill-model");
    }

    #[test]
    fn an_explicit_override_wins_over_everything_else() {
        let conn = migrated_conn();
        insert_model(&conn, "role-model", "openai", "gpt-4.1", false);
        insert_model(&conn, "skill-model", "google", "gemini-2.5-pro", false);
        insert_model(&conn, "override-model", "openrouter", "meta-llama/llama-3.1-70b-instruct", false);
        settings_repo::set(&conn, ModelRole::Coder.setting_key(), "role-model").expect("set role default");

        let resolved = resolve_model_config(&conn, ModelRole::Coder, Some("override-model"), Some("skill-model")).expect("resolve");
        assert_eq!(resolved.id, "override-model");
    }

    /// A stale id (the setting/skill still names a `model_configs.id` that
    /// no longer exists — a deleted model config) must fall through to the
    /// next tier exactly like "not set", never an error.
    #[test]
    fn a_stale_role_default_id_falls_through_to_the_global_default() {
        let conn = migrated_conn();
        settings_repo::set(&conn, ModelRole::Coder.setting_key(), "deleted-model-id").expect("set role default");

        let resolved = resolve_model_config(&conn, ModelRole::Coder, None, None).expect("resolve");
        assert_eq!(resolved.id, "default-sonnet");
    }

    #[test]
    fn a_stale_skill_preference_falls_through_past_a_real_role_default() {
        let conn = migrated_conn();
        insert_model(&conn, "role-model", "openai", "gpt-4.1", false);
        settings_repo::set(&conn, ModelRole::Coder.setting_key(), "role-model").expect("set role default");

        let resolved = resolve_model_config(&conn, ModelRole::Coder, None, Some("deleted-skill-model")).expect("resolve");
        assert_eq!(resolved.id, "role-model");
    }

    #[test]
    fn a_stale_explicit_override_falls_through_to_the_skill_preference() {
        let conn = migrated_conn();
        insert_model(&conn, "skill-model", "google", "gemini-2.5-pro", false);

        let resolved = resolve_model_config(&conn, ModelRole::Coder, Some("deleted-override"), Some("skill-model")).expect("resolve");
        assert_eq!(resolved.id, "skill-model");
    }

    /// The pathological case: no default row at all (every prior
    /// milestone's own ad-hoc fallback already assumed this exact value —
    /// this test pins that this function keeps that same promise).
    #[test]
    fn falls_back_to_the_hardcoded_default_when_model_configs_has_no_default_row() {
        let conn = migrated_conn();
        conn.execute("DELETE FROM model_configs", []).expect("empty model_configs");

        let resolved = resolve_model_config(&conn, ModelRole::Orchestrator, None, None).expect("resolve");
        assert_eq!(resolved.model_id, FALLBACK_MODEL_ID);
        assert_eq!(resolved.provider, FALLBACK_PROVIDER);
    }

    #[test]
    fn each_role_has_its_own_distinct_setting_key() {
        let keys = [
            ModelRole::Orchestrator.setting_key(),
            ModelRole::Coder.setting_key(),
            ModelRole::Reviewer.setting_key(),
            ModelRole::Utility.setting_key(),
        ];
        let mut unique = keys.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len());
    }
}

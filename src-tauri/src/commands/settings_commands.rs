//! Commands backing the Settings route: generic key/value settings via
//! `db::repository::settings`, API keys via `secrets::mod` (OS keychain —
//! never SQLite), and read-only model config listing via
//! `db::repository::model_configs`.

use std::collections::HashMap;

use tauri::{AppHandle, Manager};

use crate::commands::run_blocking;
use crate::db::models::ModelConfig;
use crate::db::repository::{model_configs as model_configs_repo, settings as settings_repo};
use crate::error::{AppError, AppResult};
use crate::secrets;
use crate::state::AppState;

/// The stored value for `key`, or `None` if it's never been set.
#[tauri::command]
pub async fn get_setting(app: AppHandle, key: String) -> Result<Option<String>, String> {
    run_blocking(move || -> AppResult<Option<String>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        let setting = settings_repo::get(&conn, &key)?;
        Ok(setting.map(|s| s.value))
    })
    .await
}

/// Inserts or updates the value for `key`.
#[tauri::command]
pub async fn set_setting(app: AppHandle, key: String, value: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        settings_repo::set(&conn, &key, &value)
    })
    .await
}

/// All settings currently stored, as a flat `key -> value` map.
#[tauri::command]
pub async fn list_settings(app: AppHandle) -> Result<HashMap<String, String>, String> {
    run_blocking(move || -> AppResult<HashMap<String, String>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        settings_repo::get_all(&conn)
    })
    .await
}

/// Stores the Anthropic API key in the OS keychain (never SQLite). Rejects
/// empty/whitespace-only input rather than silently storing garbage.
/// Runs on the blocking pool since `keyring` calls are synchronous OS calls.
#[tauri::command]
pub async fn set_api_key(key: String) -> Result<(), String> {
    run_blocking(move || -> AppResult<()> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput(
                "API key cannot be empty".to_string(),
            ));
        }
        secrets::set_secret(secrets::ANTHROPIC_API_KEY, trimmed)
    })
    .await
}

/// Whether an Anthropic API key is currently stored. Never returns the key
/// itself — only a boolean.
#[tauri::command]
pub async fn has_api_key() -> Result<bool, String> {
    run_blocking(move || -> AppResult<bool> {
        Ok(secrets::get_secret(secrets::ANTHROPIC_API_KEY)?.is_some())
    })
    .await
}

/// Removes the stored Anthropic API key, if any.
#[tauri::command]
pub async fn clear_api_key() -> Result<(), String> {
    run_blocking(move || -> AppResult<()> { secrets::delete_secret(secrets::ANTHROPIC_API_KEY) }).await
}

/// The known model configs (seeded with one Claude Sonnet 5 default),
/// default first. Read-only for now — adding/editing models is a later
/// milestone.
#[tauri::command]
pub async fn list_model_configs(app: AppHandle) -> Result<Vec<ModelConfig>, String> {
    run_blocking(move || -> AppResult<Vec<ModelConfig>> {
        let state = app.state::<AppState>();
        let conn = state.db.get()?;
        model_configs_repo::list(&conn)
    })
    .await
}

//! Commands backing the Settings route's backend-dependent sections (API
//! keys via `secrets::mod`, model config via `db::repository::model_configs`).

use tauri::State;

use crate::db::models::Setting;
use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn get_setting(_state: State<AppState>, _key: String) -> AppResult<Option<Setting>> {
    todo!("M4: db::repository::settings::get")
}

#[tauri::command]
pub fn set_setting(_state: State<AppState>, _key: String, _value: String) -> AppResult<()> {
    todo!("M4: db::repository::settings::set")
}

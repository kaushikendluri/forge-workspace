//! Commands backing the Files tab: directory listings and file reads scoped
//! to an open project's repository root.

use crate::error::AppResult;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[tauri::command]
pub fn list_directory(_repository_id: String, _relative_path: Option<String>) -> AppResult<Vec<DirEntry>> {
    todo!("M2: read_dir scoped under the repository root, rejecting path traversal")
}

#[tauri::command]
pub fn read_file(_repository_id: String, _relative_path: String) -> AppResult<String> {
    todo!("M2: read a UTF-8 file scoped under the repository root")
}

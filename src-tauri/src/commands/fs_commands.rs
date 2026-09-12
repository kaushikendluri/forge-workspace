//! Commands backing the Files tab: directory listings and file previews.
//! Paths are absolute, rooted at whatever the frontend passes in (normally
//! the active project's repository root, or a subdirectory of it) — there's
//! no repository-id indirection here, matching how `git_commands.rs` takes
//! `repo_path` directly.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use serde::Serialize;

use crate::commands::run_blocking;
use crate::error::AppResult;

/// One entry in a directory listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryDto {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// `None` for directories.
    pub size_bytes: Option<u64>,
}

/// A (possibly truncated) preview of a file's contents.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreviewDto {
    /// Empty when `is_binary` is true — binary content is never decoded.
    pub content: String,
    /// True if the file is larger than the requested `max_bytes` and was
    /// cut off.
    pub truncated: bool,
    /// True if a NUL byte was found in the bytes actually read, in which
    /// case `content` is left empty rather than attempting a lossy UTF-8
    /// decode of binary data.
    pub is_binary: bool,
}

/// Lists the contents of `dir_path` (directories first, then alphabetical,
/// case-insensitively; `.git` is skipped). Individual entries whose metadata
/// can't be read are skipped rather than failing the whole listing — only a
/// failure to read `dir_path` itself is an error.
#[tauri::command]
pub async fn list_directory(dir_path: String) -> Result<Vec<DirEntryDto>, String> {
    run_blocking(move || -> AppResult<Vec<DirEntryDto>> {
        let dir = PathBuf::from(&dir_path);
        let read_dir = fs::read_dir(&dir)?;

        let mut entries = Vec::new();
        for entry in read_dir {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let Ok(metadata) = entry.metadata() else { continue };
            let is_dir = metadata.is_dir();
            entries.push(DirEntryDto {
                name,
                path: entry.path().to_string_lossy().into_owned(),
                is_dir,
                size_bytes: if is_dir { None } else { Some(metadata.len()) },
            });
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        Ok(entries)
    })
    .await
}

/// Reads up to `max_bytes` of `file_path`, reporting whether it looks binary
/// (a NUL byte in what was read) and whether the file was longer than
/// `max_bytes` (truncated). Never attempts to decode binary content as
/// UTF-8 — a huge or binary file is reported honestly rather than crashing
/// or dumping garbage.
#[tauri::command]
pub async fn read_file_preview(file_path: String, max_bytes: u32) -> Result<FilePreviewDto, String> {
    run_blocking(move || -> AppResult<FilePreviewDto> {
        let path = PathBuf::from(&file_path);
        let metadata = fs::metadata(&path)?;
        let cap = max_bytes as u64;

        let file = fs::File::open(&path)?;
        let mut buf = Vec::with_capacity(cap.min(metadata.len()) as usize);
        file.take(cap).read_to_end(&mut buf)?;

        let is_binary = buf.contains(&0);
        let truncated = metadata.len() > buf.len() as u64;

        if is_binary {
            return Ok(FilePreviewDto { content: String::new(), truncated, is_binary: true });
        }

        Ok(FilePreviewDto {
            content: String::from_utf8_lossy(&buf).into_owned(),
            truncated,
            is_binary: false,
        })
    })
    .await
}

//! Security boundary for every filesystem tool (`read_file`, `write_file`,
//! `edit_file`, `list_directory`, `search_files`, `search_code`): resolves a
//! model-supplied path against a run's workspace root and refuses anything
//! that would resolve outside it.
//!
//! This is the actual line between "an AI agent with file access" and "an AI
//! agent that can only touch its own sandboxed git worktree" — every check
//! here is deliberately a hard rejection (a `tool_result` error string the
//! model sees and can react to), never a silent clamp/rewrite into something
//! "close enough" inside the root.

use std::path::{Component, Path, PathBuf};

/// Resolves `user_path` (as supplied by the model, untrusted) against
/// `workspace_root`, rejecting:
/// - any absolute path (Unix `/...`, Windows `C:\...` or `\\...`),
/// - any path containing a `..` component,
/// - any path that — once symlinks are resolved — lands outside
///   `workspace_root` (e.g. a symlink planted inside the workspace that
///   points outside it).
///
/// Backslashes are treated as path separators regardless of host OS (not
/// just on Windows): tool input is a string from the model, not a
/// platform-native path, and a `..\..\` escape attempt must be rejected the
/// same way whether this binary happens to be running on Windows or macOS —
/// tests for exactly this run on both.
///
/// The target need not exist yet (`write_file` creates new files/dirs): the
/// deepest existing ancestor is canonicalized and checked, and the
/// not-yet-existing remainder is re-appended on top of that canonical,
/// verified-inside-root prefix.
pub fn resolve_in_workspace(workspace_root: &Path, user_path: &str) -> Result<PathBuf, String> {
    let normalized = user_path.replace('\\', "/");
    let rel = Path::new(&normalized);

    for component in rel.components() {
        match component {
            Component::ParentDir => {
                return Err(format!(
                    "path '{user_path}' may not contain '..' — it must stay inside the workspace"
                ));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(format!(
                    "path '{user_path}' must be relative to the workspace root, not absolute"
                ));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|e| format!("failed to resolve workspace root: {e}"))?;

    let joined = canonical_root.join(rel);

    // Walk up to the deepest existing ancestor so a not-yet-existing target
    // (a new file, or new nested directories) can still be canonicalized and
    // checked via its existing prefix.
    let mut existing: &Path = joined.as_path();
    let mut remainder: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                remainder.push(name.to_os_string());
                existing = parent;
            }
            _ => break,
        }
    }

    let canonical_existing = existing
        .canonicalize()
        .map_err(|e| format!("failed to resolve path '{user_path}': {e}"))?;

    if !canonical_existing.starts_with(&canonical_root) {
        return Err(format!(
            "path '{user_path}' escapes the workspace root"
        ));
    }

    let mut result = canonical_existing;
    for part in remainder.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn resolves_plain_relative_path_inside_workspace() {
        let ws = workspace();
        std::fs::write(ws.path().join("file.txt"), "hi").expect("write");
        let resolved = resolve_in_workspace(ws.path(), "file.txt").expect("should resolve");
        assert_eq!(resolved, ws.path().canonicalize().unwrap().join("file.txt"));
    }

    #[test]
    fn resolves_new_nested_path_that_does_not_exist_yet() {
        let ws = workspace();
        let resolved = resolve_in_workspace(ws.path(), "src/new/file.txt").expect("should resolve");
        assert_eq!(
            resolved,
            ws.path().canonicalize().unwrap().join("src").join("new").join("file.txt")
        );
    }

    /// Unix-style traversal escape — the exact pattern named in the M6 spec.
    #[test]
    fn rejects_unix_style_parent_traversal() {
        let ws = workspace();
        let err = resolve_in_workspace(ws.path(), "../../../etc/passwd").expect_err("must be rejected");
        assert!(err.contains(".."), "error should explain the '..' rejection: {err}");
    }

    /// Windows-style traversal escape — rejected the same way regardless of
    /// which OS this test binary is actually running on, since backslashes
    /// are normalized to separators unconditionally.
    #[test]
    fn rejects_windows_style_parent_traversal() {
        let ws = workspace();
        let err =
            resolve_in_workspace(ws.path(), "..\\..\\Windows\\System32").expect_err("must be rejected");
        assert!(err.contains(".."), "error should explain the '..' rejection: {err}");
    }

    #[test]
    fn rejects_absolute_unix_path() {
        let ws = workspace();
        let err = resolve_in_workspace(ws.path(), "/etc/passwd").expect_err("must be rejected");
        assert!(err.contains("absolute"), "error should explain the absolute-path rejection: {err}");
    }

    #[test]
    fn rejects_absolute_windows_drive_path() {
        let ws = workspace();
        // Backslash-normalization turns this into `C:/Windows/System32`,
        // which `Path`'s prefix/root detection recognizes as absolute on
        // Windows; on non-Windows hosts `C:` merely isn't a real escape
        // (there is no such drive concept), so this case is authoritative
        // on Windows CI specifically while remaining harmless everywhere
        // else.
        if cfg!(windows) {
            let err = resolve_in_workspace(ws.path(), "C:\\Windows\\System32").expect_err("must be rejected");
            assert!(err.contains("absolute"), "error should explain the absolute-path rejection: {err}");
        }
    }

    #[test]
    fn rejects_unc_style_absolute_path() {
        let ws = workspace();
        if cfg!(windows) {
            let err = resolve_in_workspace(ws.path(), "\\\\server\\share\\secret").expect_err("must be rejected");
            assert!(err.contains("absolute"), "error should explain the absolute-path rejection: {err}");
        }
    }

    #[test]
    fn rejects_traversal_hidden_after_a_normal_looking_prefix() {
        let ws = workspace();
        let err = resolve_in_workspace(ws.path(), "src/../../outside").expect_err("must be rejected");
        assert!(err.contains(".."), "error should explain the '..' rejection: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_that_escapes_the_workspace_root() {
        let ws = workspace();
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::write(outside.path().join("secret.txt"), "top secret").expect("write");
        std::os::unix::fs::symlink(outside.path(), ws.path().join("escape")).expect("symlink");

        let err = resolve_in_workspace(ws.path(), "escape/secret.txt").expect_err("must be rejected");
        assert!(err.contains("escapes"), "error should explain the escape rejection: {err}");
    }
}

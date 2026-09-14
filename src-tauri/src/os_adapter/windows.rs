use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use super::OperatingSystemAdapter;

pub struct WindowsAdapter;

impl OperatingSystemAdapter for WindowsAdapter {
    fn default_shell(&self) -> String {
        // Prefer PowerShell 7+ (`pwsh.exe`) when it's on PATH, falling back to
        // Windows PowerShell (`powershell.exe`), which ships on every Windows
        // install.
        if self.resolve_executable("pwsh").is_some() {
            "pwsh.exe".to_string()
        } else {
            "powershell.exe".to_string()
        }
    }

    fn shell_invocation(&self) -> Vec<String> {
        vec![self.default_shell(), "-NoLogo".to_string()]
    }

    fn one_shot_shell_invocation(&self, command: &str) -> Vec<String> {
        vec![
            self.default_shell(),
            "-NoLogo".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            command.to_string(),
        ]
    }

    fn resolve_executable(&self, name: &str) -> Option<PathBuf> {
        let path_var = env::var_os("PATH")?;
        let candidate_exts = ["", ".exe", ".cmd", ".bat"];
        for dir in env::split_paths(&path_var) {
            for ext in candidate_exts {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn env_vars(&self) -> HashMap<String, String> {
        env::vars().collect()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        env::var_os("USERPROFILE").map(PathBuf::from)
    }
}

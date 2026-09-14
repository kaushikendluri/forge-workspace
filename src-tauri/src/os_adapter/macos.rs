use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use super::OperatingSystemAdapter;

pub struct MacOsAdapter;

impl OperatingSystemAdapter for MacOsAdapter {
    fn default_shell(&self) -> String {
        env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    }

    fn shell_invocation(&self) -> Vec<String> {
        // `-i` (interactive) and `-l` (login) so the user's shell rc files
        // (and therefore their PATH, nvm/rustup shims, etc.) are loaded.
        vec![self.default_shell(), "-il".to_string()]
    }

    fn one_shot_shell_invocation(&self, command: &str) -> Vec<String> {
        // `-l` (login, for PATH/nvm/rustup shims) + `-c` (run one command
        // and exit) rather than `-i` — an interactive shell would otherwise
        // wait on a tty that's never attached.
        vec![self.default_shell(), "-lc".to_string(), command.to_string()]
    }

    fn resolve_executable(&self, name: &str) -> Option<PathBuf> {
        let path_var = env::var_os("PATH")?;
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    fn env_vars(&self) -> HashMap<String, String> {
        env::vars().collect()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        env::var_os("HOME").map(PathBuf::from)
    }
}

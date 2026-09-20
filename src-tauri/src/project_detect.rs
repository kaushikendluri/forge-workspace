//! M12: detects sensible default `test_command`/`lint_command`/`build_command`
//! values for a repo root by inspecting its own real project files —
//! `package.json` scripts, `Cargo.toml`, `go.mod`, `pyproject.toml`/
//! `requirements.txt`. This only *pre-fills* the per-project `settings` a
//! user can see and override (via `commands::project_commands::open_project`/
//! `init_project`, and the Testing tab) — it never runs anything itself, and
//! it never guesses: a command this can't name with real confidence (e.g. a
//! Python lint/build step, since there's no single dominant convention) is
//! left `None` rather than filled with a plausible-looking default. This
//! mirrors `agent::tools::run_configured_command`'s own behavior of erroring
//! clearly on an unset command rather than ever fabricating one.

use std::path::Path;

/// The commands detected for one repo root. Any field may be `None` if
/// nothing about that root confidently implies a command for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectedCommands {
    pub test_command: Option<String>,
    pub lint_command: Option<String>,
    pub build_command: Option<String>,
}

/// The three `settings` keys a project's detected/configured commands live
/// under: `project.<project_id>.<test|lint|build>_command`. Kept as one
/// helper so every caller (the M12 detection wiring, `agent::tool_loop`'s
/// settings lookup, and the Testing tab's commands) agrees on the exact
/// format. `kind` is `"test_command"` / `"lint_command"` / `"build_command"`.
pub fn project_setting_key(project_id: &str, kind: &str) -> String {
    format!("project.{project_id}.{kind}")
}

/// The companion key recording whether `project_setting_key(project_id,
/// kind)`'s value was auto-detected or set by the user — see
/// `commands::testing_commands` for how it's read/written. Absent entirely
/// (not just an empty string) when the value itself is unset.
pub fn project_setting_source_key(project_id: &str, kind: &str) -> String {
    format!("{}.source", project_setting_key(project_id, kind))
}

enum NodePackageManager {
    Npm,
    Yarn,
    Pnpm,
}

impl NodePackageManager {
    /// Inferred from lockfile presence in `root` — `package-lock.json` and
    /// "no recognized lockfile at all" both fall back to npm, since a
    /// `package.json` with no lockfile is still overwhelmingly an npm
    /// project in practice.
    fn detect(root: &Path) -> Self {
        if root.join("pnpm-lock.yaml").is_file() {
            Self::Pnpm
        } else if root.join("yarn.lock").is_file() {
            Self::Yarn
        } else {
            Self::Npm
        }
    }

    /// `npm test` has a documented shorthand that runs the `test` script
    /// directly; yarn and pnpm both support the same bare form.
    fn test(&self) -> String {
        match self {
            Self::Npm => "npm test".to_string(),
            Self::Yarn => "yarn test".to_string(),
            Self::Pnpm => "pnpm test".to_string(),
        }
    }

    /// Every other script name needs the explicit `run` subcommand — there's
    /// no universal bare-name shorthand for arbitrary script names across
    /// all three package managers.
    fn run(&self, script: &str) -> String {
        match self {
            Self::Npm => format!("npm run {script}"),
            Self::Yarn => format!("yarn run {script}"),
            Self::Pnpm => format!("pnpm run {script}"),
        }
    }
}

/// `Some` (possibly with every field still `None`, if `package.json` exists
/// but defines none of `test`/`lint`/`build`) when `root` looks like a
/// Node project at all; `None` if there's no `package.json` here.
fn detect_node(root: &Path) -> Option<DetectedCommands> {
    let raw = std::fs::read_to_string(root.join("package.json")).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let scripts = parsed.get("scripts").and_then(serde_json::Value::as_object);
    let has_script = |name: &str| scripts.map(|s| s.contains_key(name)).unwrap_or(false);
    let pm = NodePackageManager::detect(root);

    Some(DetectedCommands {
        test_command: has_script("test").then(|| pm.test()),
        lint_command: has_script("lint").then(|| pm.run("lint")),
        build_command: has_script("build").then(|| pm.run("build")),
    })
}

/// Rust's tooling is uniform enough (unlike Node's script-name conventions)
/// that `Cargo.toml`'s mere presence is confidence enough for all three.
fn detect_rust(root: &Path) -> Option<DetectedCommands> {
    if !root.join("Cargo.toml").is_file() {
        return None;
    }
    Some(DetectedCommands {
        test_command: Some("cargo test".to_string()),
        lint_command: Some("cargo clippy".to_string()),
        build_command: Some("cargo build".to_string()),
    })
}

/// `go vet`/third-party linters aren't universally installed the way `cargo
/// clippy` ships with every Rust toolchain, so — same rule as Python below —
/// lint is left unset rather than guessed.
fn detect_go(root: &Path) -> Option<DetectedCommands> {
    if !root.join("go.mod").is_file() {
        return None;
    }
    Some(DetectedCommands {
        test_command: Some("go test ./...".to_string()),
        lint_command: None,
        build_command: Some("go build ./...".to_string()),
    })
}

/// Python has no single dominant lint (`ruff`/`flake8`/`pylint`) or build
/// convention, so only `test_command` is ever filled in, via `pytest` — by
/// far the most common test runner for a project shaped like this.
fn detect_python(root: &Path) -> Option<DetectedCommands> {
    if !root.join("pyproject.toml").is_file() && !root.join("requirements.txt").is_file() {
        return None;
    }
    Some(DetectedCommands { test_command: Some("pytest".to_string()), lint_command: None, build_command: None })
}

/// Inspects `root`'s own files — no recursion into subdirectories — and
/// returns the first matching ecosystem's commands, in this precedence:
/// Node (`package.json`) > Rust (`Cargo.toml`) > Go (`go.mod`) > Python
/// (`pyproject.toml`/`requirements.txt`). A root with signals from more than
/// one ecosystem (e.g. this very repo, whose Tauri backend's `Cargo.toml`
/// lives in `src-tauri/` rather than the root) resolves to exactly one
/// ecosystem's commands rather than merging across them — pairing "npm
/// test" with "cargo clippy" under one project's settings would be
/// misleading, not more helpful. Returns `DetectedCommands::default()`
/// (every field `None`) when nothing recognizable is found at all.
pub fn detect_commands(root: &Path) -> DetectedCommands {
    detect_node(root)
        .or_else(|| detect_rust(root))
        .or_else(|| detect_go(root))
        .or_else(|| detect_python(root))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn empty_directory_detects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_commands(dir.path()), DetectedCommands::default());
    }

    #[test]
    fn package_json_with_full_scripts_and_no_lockfile_defaults_to_npm() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"scripts":{"test":"vitest","lint":"eslint .","build":"tsc"}}"#);
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("npm test"));
        assert_eq!(detected.lint_command.as_deref(), Some("npm run lint"));
        assert_eq!(detected.build_command.as_deref(), Some("npm run build"));
    }

    #[test]
    fn package_json_with_yarn_lock_uses_yarn() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"scripts":{"test":"jest","lint":"eslint ."}}"#);
        write(dir.path(), "yarn.lock", "");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("yarn test"));
        assert_eq!(detected.lint_command.as_deref(), Some("yarn run lint"));
        assert_eq!(detected.build_command, None, "no build script means no build command, not a guess");
    }

    #[test]
    fn package_json_with_pnpm_lock_uses_pnpm() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"scripts":{"build":"vite build"}}"#);
        write(dir.path(), "pnpm-lock.yaml", "");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.build_command.as_deref(), Some("pnpm run build"));
        assert_eq!(detected.test_command, None);
        assert_eq!(detected.lint_command, None);
    }

    #[test]
    fn package_json_with_no_scripts_key_detects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"name":"whatever"}"#);
        assert_eq!(detect_commands(dir.path()), DetectedCommands::default());
    }

    #[test]
    fn malformed_package_json_falls_through_rather_than_detecting_node() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", "{ not valid json");
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("cargo test"), "unparsable package.json should not win over a real Cargo.toml");
    }

    #[test]
    fn cargo_toml_alone_detects_the_standard_cargo_triple() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\nversion = \"0.1.0\"\n");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("cargo test"));
        assert_eq!(detected.lint_command.as_deref(), Some("cargo clippy"));
        assert_eq!(detected.build_command.as_deref(), Some("cargo build"));
    }

    #[test]
    fn go_mod_alone_detects_test_and_build_but_not_lint() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "go.mod", "module example.com/x\n\ngo 1.22\n");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("go test ./..."));
        assert_eq!(detected.build_command.as_deref(), Some("go build ./..."));
        assert_eq!(detected.lint_command, None, "no single dominant Go linter to assume");
    }

    #[test]
    fn pyproject_toml_detects_pytest_only() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "pyproject.toml", "[project]\nname = \"x\"\n");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("pytest"));
        assert_eq!(detected.lint_command, None);
        assert_eq!(detected.build_command, None);
    }

    #[test]
    fn requirements_txt_alone_also_detects_pytest() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "requirements.txt", "pytest\nrequests\n");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("pytest"));
    }

    #[test]
    fn conflicting_signals_node_takes_precedence_over_rust() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "package.json", r#"{"scripts":{"test":"vitest"}}"#);
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("npm test"), "package.json at the root should win over a Cargo.toml also present");
    }

    #[test]
    fn conflicting_signals_rust_takes_precedence_over_go_and_python() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
        write(dir.path(), "go.mod", "module example.com/x\n\ngo 1.22\n");
        write(dir.path(), "pyproject.toml", "[project]\nname = \"x\"\n");
        let detected = detect_commands(dir.path());
        assert_eq!(detected.test_command.as_deref(), Some("cargo test"));
    }

    #[test]
    fn project_setting_key_format() {
        assert_eq!(project_setting_key("proj-1", "test_command"), "project.proj-1.test_command");
        assert_eq!(project_setting_source_key("proj-1", "test_command"), "project.proj-1.test_command.source");
    }
}

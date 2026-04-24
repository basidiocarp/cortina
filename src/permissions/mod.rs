use std::fmt;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

/// Raw TOML shape for a permissions config file.
// Fields populated by serde; not directly constructed in code.
#[allow(dead_code)]
#[derive(Debug, Deserialize, Default)]
struct PermissionsConfig {
    #[serde(default)]
    allow_redirects: bool,
    #[serde(default)]
    allow: Vec<PatternEntry>,
    #[serde(default)]
    deny: Vec<PatternEntry>,
}

// Fields populated by serde; not directly constructed in code.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct PatternEntry {
    pattern: String,
}

/// Errors returned by command permission validation.
#[derive(Debug, Clone)]
pub enum PermissionError {
    /// Command is explicitly denied by a policy pattern.
    Denied { command: String, pattern: String },
    /// Command contains dangerous operators (backticks, $()).
    DangerousOperator { command: String },
    /// Command contains redirects when they are not allowed.
    RedirectNotAllowed { command: String },
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied { command, pattern } => {
                write!(f, "command denied by pattern '{pattern}': {command}")
            }
            Self::DangerousOperator { command } => {
                write!(f, "dangerous operator in command: {command}")
            }
            Self::RedirectNotAllowed { command } => {
                write!(f, "redirect not allowed: {command}")
            }
        }
    }
}

impl std::error::Error for PermissionError {}

/// Controller for validating shell command strings against permission policies.
///
/// The controller splits chained commands (&&, ||, |, ;) and validates each segment
/// independently. It can check against allow and deny patterns, detect dangerous
/// operators, and enforce restrictions on shell redirects.
#[derive(Debug, Clone, Default)]
pub struct CommandPermissionController {
    allow_patterns: Vec<String>,
    deny_patterns: Vec<String>,
    allow_redirects: bool,
}

impl CommandPermissionController {
    /// Load config from global and workspace permissions files.
    ///
    /// Reads (in order):
    /// 1. `~/.config/basidiocarp/permissions.toml` (global)
    /// 2. `.basidiocarp/permissions.toml` (workspace, relative to cwd)
    ///
    /// Patterns from both layers are merged; workspace does not replace global.
    /// Returns a default (allow-all) controller if neither file exists or can be parsed.
    #[allow(dead_code)] // API method
    #[must_use]
    pub fn load() -> Self {
        let mut allow_patterns = Vec::new();
        let mut deny_patterns = Vec::new();
        let mut allow_redirects = false;

        // Resolve global config path via $HOME rather than adding a dirs_next dep.
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("~"));
        let config_paths: Vec<PathBuf> = vec![
            PathBuf::from(&home)
                .join(".config")
                .join("basidiocarp")
                .join("permissions.toml"),
            PathBuf::from(".basidiocarp").join("permissions.toml"),
        ];

        for path in &config_paths {
            let Ok(contents) = fs::read_to_string(path) else {
                continue; // file absent or unreadable — skip silently
            };
            match toml::from_str::<PermissionsConfig>(&contents) {
                Ok(cfg) => {
                    allow_redirects |= cfg.allow_redirects;
                    for entry in cfg.allow {
                        allow_patterns.push(entry.pattern);
                    }
                    for entry in cfg.deny {
                        deny_patterns.push(entry.pattern);
                    }
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to parse permissions config");
                }
            }
        }

        CommandPermissionController {
            allow_patterns,
            deny_patterns,
            allow_redirects,
        }
    }

    /// Create a new permission controller with explicit patterns.
    ///
    /// # Arguments
    /// * `allow_patterns` — glob patterns for allowed commands (if non-empty, only these are allowed).
    /// * `deny_patterns` — glob patterns for commands to always deny.
    /// * `allow_redirects` — if false, blocks `<` and `>` in commands.
    #[allow(dead_code)] // API method
    #[must_use]
    pub fn new(
        allow_patterns: Vec<String>,
        deny_patterns: Vec<String>,
        allow_redirects: bool,
    ) -> Self {
        CommandPermissionController {
            allow_patterns,
            deny_patterns,
            allow_redirects,
        }
    }

    /// Validate a shell command string against policy.
    ///
    /// Splits chained commands (&&, ||, |, ;) and validates each segment.
    /// Returns Ok(()) if the command passes all checks, or Err with the first
    /// violation encountered.
    #[allow(dead_code)] // API method
    pub fn validate(&self, command: &str) -> Result<(), PermissionError> {
        for segment in split_chained(command) {
            let segment = segment.trim();
            if Self::has_dangerous_operators(segment) {
                return Err(PermissionError::DangerousOperator {
                    command: command.to_string(),
                });
            }
            if !self.allow_redirects && has_redirect(segment) {
                return Err(PermissionError::RedirectNotAllowed {
                    command: command.to_string(),
                });
            }
            if !self.allow_patterns.is_empty() {
                let allowed = self.allow_patterns.iter().any(|p| glob_matches(p, segment));
                if !allowed {
                    return Err(PermissionError::Denied {
                        command: command.to_string(),
                        pattern: "(no allow pattern matched)".to_string(),
                    });
                }
            }
            for deny_pat in &self.deny_patterns {
                if glob_matches(deny_pat, segment) {
                    return Err(PermissionError::Denied {
                        command: command.to_string(),
                        pattern: deny_pat.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn has_dangerous_operators(segment: &str) -> bool {
        // Backtick substitution, $() substitution
        segment.contains('`') || segment.contains("$(")
    }
}

/// Split a shell command string on &&, ||, |, ; (simple left-to-right split).
#[allow(dead_code)] // Helper for API
fn split_chained(cmd: &str) -> Vec<&str> {
    // Simple approach: split on these operator strings
    let mut parts = vec![cmd];
    for op in &["&&", "||", "|", ";"] {
        parts = parts.into_iter().flat_map(|p| p.split(op)).collect();
    }
    parts
}

#[allow(dead_code)] // Helper for API
fn has_redirect(segment: &str) -> bool {
    segment.contains('>') || segment.contains('<')
}

/// Simple glob matching: '*' matches any sequence.
#[allow(dead_code)] // Helper for API
fn glob_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        value.starts_with(prefix)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        value.ends_with(suffix)
    } else {
        pattern == value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_allows_all() {
        let controller = CommandPermissionController::default();
        assert!(controller.validate("cargo test").is_ok());
        assert!(controller.validate("git status").is_ok());
    }

    #[test]
    fn deny_pattern_blocks_command() {
        let controller =
            CommandPermissionController::new(vec![], vec!["rm -rf *".to_string()], false);
        assert!(controller.validate("rm -rf /tmp/test").is_err());
    }

    #[test]
    fn backtick_is_dangerous() {
        let controller = CommandPermissionController::default();
        assert!(controller.validate("echo `whoami`").is_err());
    }

    #[test]
    fn dollar_paren_is_dangerous() {
        let controller = CommandPermissionController::default();
        assert!(controller.validate("echo $(whoami)").is_err());
    }

    #[test]
    fn redirect_blocked_by_default() {
        let controller = CommandPermissionController::default();
        assert!(
            controller
                .validate("cargo build > /tmp/output.txt")
                .is_err()
        );
        assert!(controller.validate("cat < /tmp/input.txt").is_err());
    }

    #[test]
    fn allow_redirects_permits_them() {
        let controller = CommandPermissionController::new(vec![], vec![], true);
        assert!(controller.validate("cargo build > /tmp/output.txt").is_ok());
    }

    #[test]
    fn allow_pattern_restricts_commands() {
        let controller = CommandPermissionController::new(
            vec!["cargo *".to_string(), "git *".to_string()],
            vec![],
            false,
        );
        assert!(controller.validate("cargo test").is_ok());
        assert!(controller.validate("git status").is_ok());
        assert!(controller.validate("rm /tmp/file").is_err());
    }

    #[test]
    fn deny_pattern_overrides_allow() {
        let controller = CommandPermissionController::new(
            vec!["cargo *".to_string()],
            vec!["cargo clean".to_string()],
            false,
        );
        assert!(controller.validate("cargo test").is_ok());
        assert!(controller.validate("cargo clean").is_err());
    }

    #[test]
    fn chained_commands_validated_per_segment() {
        let controller = CommandPermissionController::new(
            vec!["cargo *".to_string(), "echo *".to_string()],
            vec![],
            false,
        );
        assert!(controller.validate("cargo test && echo done").is_ok());
        assert!(controller.validate("cargo test && rm /tmp/file").is_err());
    }

    #[test]
    fn glob_matches_prefix() {
        assert!(glob_matches("cargo*", "cargo test"));
        assert!(glob_matches("cargo*", "cargo"));
        assert!(!glob_matches("cargo*", "git status"));
    }

    #[test]
    fn glob_matches_suffix() {
        assert!(glob_matches("*.txt", "file.txt"));
        assert!(glob_matches("*.txt", ".txt"));
        assert!(!glob_matches("*.txt", "file.rs"));
    }

    #[test]
    fn glob_matches_exact() {
        assert!(glob_matches("exact", "exact"));
        assert!(!glob_matches("exact", "not-exact"));
    }

    #[test]
    fn glob_matches_wildcard() {
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("*", ""));
    }
}

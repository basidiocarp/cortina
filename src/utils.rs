// ─────────────────────────────────────────────────────────────────────────
// Shared utilities for hook handlers
// ─────────────────────────────────────────────────────────────────────────

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

// ─────────────────────────────────────────────────────────────────────────
// Importance levels for stored content
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum Importance {
    #[allow(dead_code, reason = "Reserved for future use")]
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub project: String,
}

impl Importance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Check if a binary exists in PATH
pub fn command_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

/// Get a stable hash of the current working directory for temp file isolation
pub fn cwd_hash() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let cwd = env::current_dir().map_or_else(
        |_| env::temp_dir().to_string_lossy().to_string(),
        |p| p.to_string_lossy().to_string(),
    );

    let mut hasher = DefaultHasher::new();
    cwd.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Resolve a temp state file path for Cortina tracking state.
pub fn temp_state_path(name: &str, hash: &str, extension: &str) -> PathBuf {
    env::temp_dir().join(format!("cortina-{name}-{hash}.{extension}"))
}

fn session_state_path(hash: &str) -> PathBuf {
    temp_state_path("session", hash, "json")
}

/// Get project name from current working directory
pub fn get_project_name() -> Option<String> {
    env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

/// Normalize a command for tracking (e.g., "cargo test --lib" -> "cargo test")
pub fn normalize_command(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.len() >= 2 {
        format!("{} {}", parts[0], parts[1])
    } else if !parts.is_empty() {
        parts[0].to_string()
    } else {
        cmd.to_string()
    }
}

/// Detect if output contains error patterns
pub fn has_error(output: &str, exit_code: Option<i32>) -> bool {
    // Check exit code
    if let Some(code) = exit_code {
        if code != 0 {
            return true;
        }
    }

    // Check for error patterns
    for re in error_patterns() {
        if re.is_match(output) {
            return true;
        }
    }

    false
}

fn error_patterns() -> &'static [Regex] {
    static ERROR_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    ERROR_PATTERNS.get_or_init(|| {
        [
            r"\berror[\s:[\]]",
            r"\bFAILED\b",
            r"\bpanicked\b",
            r"\bfailed\b",
            r"\bfatal[\s:]",
            r"\bcommand not found\b",
            r"\bsegmentation fault\b",
            r"\baborted\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Check if a file path is a document file (markdown, config, etc.)
pub fn is_document_file(path: &str) -> bool {
    let extensions = [
        ".md", ".txt", ".rst", ".adoc", // Documentation
        ".json", ".yaml", ".yml", ".toml", // Config/Data
        ".html", ".css", // Web
        ".env", ".cfg", ".ini", // Environment/Config
        ".sh", ".sql", // Scripts/Queries
    ];

    let path_lower = path.to_lowercase();
    extensions.iter().any(|ext| path_lower.ends_with(ext))
}

/// Check if a command is a build command
pub fn is_build_command(cmd: &str) -> bool {
    for re in build_patterns() {
        if re.is_match(cmd) {
            return true;
        }
    }

    false
}

fn build_patterns() -> &'static [Regex] {
    static BUILD_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    BUILD_PATTERNS.get_or_init(|| {
        [
            r"\bcargo\s+(build|check)\b",
            r"\bnpm\s+run\s+build\b",
            r"\byarn\s+build\b",
            r"\bpnpm\s+build\b",
            r"\bbun\s+build\b",
            r"\btsc\b",
            r"\bnext\s+build\b",
            r"\bmake\b",
            r"\bgo\s+build\b",
            r"\bgradlew\s+build\b",
            r"\bmvn\s+clean\s+package\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Check if a command is a significant (worth tracking) command
pub fn is_significant_command(cmd: &str) -> bool {
    for re in significant_patterns() {
        if re.is_match(cmd) {
            return true;
        }
    }

    false
}

fn significant_patterns() -> &'static [Regex] {
    static SIG_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    SIG_PATTERNS.get_or_init(|| {
        [
            r"\bcargo\b",
            r"\bnpm\b",
            r"\byarn\b",
            r"\bpnpm\b",
            r"\bbun\b",
            r"\bgit\s+push\b",
            r"\bdocker\b",
            r"\bpytest\b",
            r"\bmake\b",
            r"\bgo\s+(build|test|run|vet)\b",
            r"\brustc\b",
            r"\bgcc\b",
            r"\bg\+\+\b",
            r"\bjavac\b",
            r"\bmvn\b",
            r"\bgradle\b",
            r"\bvitest\b",
            r"\bjest\b",
            r"\bplaywright\b",
            r"\btsc\b",
            r"\bpython\b",
            r"\bruby\b",
            r"\bswift\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Load JSON from a temp file
pub fn load_json_file<T: serde::de::DeserializeOwned>(path: impl AsRef<Path>) -> Option<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
}

/// Save JSON to a temp file
pub fn save_json_file<T: serde::Serialize>(path: impl AsRef<Path>, data: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(data)?;
    fs::write(path, json)?;
    Ok(())
}

pub fn load_session_state(hash: &str) -> Option<SessionState> {
    load_json_file(session_state_path(hash))
}

pub fn clear_session_state(hash: &str) {
    let _ = fs::remove_file(session_state_path(hash));
}

pub fn ensure_hyphae_session(project: &str, task: Option<&str>) -> Option<SessionState> {
    if !command_exists("hyphae") {
        return None;
    }

    ensure_hyphae_session_with(project, task, Command::output)
}

fn ensure_hyphae_session_with<F>(
    project: &str,
    task: Option<&str>,
    mut run_command: F,
) -> Option<SessionState>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let hash = cwd_hash();
    if let Some(existing) = load_session_state(&hash) {
        return Some(existing);
    }

    let mut cmd = Command::new("hyphae");
    cmd.args(["session", "start", "--project", project]);
    if let Some(task_desc) = task.filter(|value| !value.trim().is_empty()) {
        cmd.args(["--task", task_desc]);
    }

    let output = run_command(&mut cmd).ok()?;
    if !output.status.success() {
        return None;
    }

    let session_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if session_id.is_empty() {
        return None;
    }

    let state = SessionState {
        session_id,
        project: project.to_string(),
    };
    save_json_file(session_state_path(&hash), &state).ok()?;
    Some(state)
}

pub fn end_hyphae_session(
    summary: Option<&str>,
    files_modified: &[String],
    errors_encountered: usize,
) -> bool {
    if !command_exists("hyphae") {
        return false;
    }

    end_hyphae_session_with(
        cwd_hash().as_str(),
        summary,
        files_modified,
        errors_encountered,
        Command::output,
    )
}

fn end_hyphae_session_with<F>(
    hash: &str,
    summary: Option<&str>,
    files_modified: &[String],
    errors_encountered: usize,
    mut run_command: F,
) -> bool
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    if hash.is_empty() {
        return false;
    }

    let Some(state) = load_session_state(hash) else {
        return false;
    };

    let mut cmd = Command::new("hyphae");
    cmd.args(["session", "end", "--id", &state.session_id]);

    if let Some(summary_text) = summary.filter(|value| !value.trim().is_empty()) {
        cmd.args(["--summary", summary_text]);
    }

    for file in files_modified {
        cmd.args(["--file", file]);
    }

    cmd.args(["--errors", &errors_encountered.to_string()]);

    let Ok(output) = run_command(&mut cmd) else {
        return false;
    };

    if output.status.success() {
        clear_session_state(hash);
        return true;
    }

    false
}

pub fn log_hyphae_feedback_signal(signal_type: &str, signal_value: i64, source: &str) {
    if !command_exists("hyphae") {
        return;
    }

    let hash = cwd_hash();
    let Some(state) = load_session_state(&hash) else {
        return;
    };

    let mut cmd = Command::new("hyphae");
    cmd.args(["feedback", "signal"])
        .args(["--session-id", &state.session_id])
        .args(["--type", signal_type])
        .args(["--value", &signal_value.to_string()])
        .args(["--source", source])
        .args(["--project", &state.project]);

    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Store content in Hyphae (fire and forget)
pub fn store_in_hyphae(topic: &str, content: &str, importance: Importance, project: Option<&str>) {
    if !command_exists("hyphae") {
        return;
    }

    let mut cmd = Command::new("hyphae");
    cmd.args(["store", "--topic", topic])
        .args(["--content", content])
        .args(["--importance", importance.as_str()])
        .args(["--keywords", "cortina,hook"]);

    if let Some(proj) = project {
        cmd.args(["-P", proj]);
    }

    // Fire and forget — spawn without waiting
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Spawn a command asynchronously (fire and forget)
pub fn spawn_async(cmd: &str, args: &[&str]) {
    let mut command = Command::new(cmd);
    for arg in args {
        command.arg(arg);
    }

    let _ = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    fn output_with_status(code: i32, stdout: &str) -> Output {
        Output {
            status: exit_status_from_code(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[cfg(unix)]
    fn exit_status_from_code(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit_status_from_code(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    // ─────────────────────────────────────────────────────────────────────
    // has_error tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_has_error_with_non_zero_exit_code() {
        assert!(has_error("", Some(1)));
        assert!(has_error("anything", Some(127)));
        assert!(has_error("", Some(-1)));
    }

    #[test]
    fn test_has_error_with_zero_exit_code_and_no_error_patterns() {
        assert!(!has_error("Success", Some(0)));
        assert!(!has_error("completed successfully", Some(0)));
    }

    #[test]
    fn test_has_error_with_error_pattern_in_output() {
        // Pattern: \bfailed\b
        assert!(
            has_error("Command failed", Some(0)),
            "Should detect 'failed'"
        );
        // Pattern: \bFAILED\b
        assert!(
            has_error("FAILED: test suite", Some(0)),
            "Should detect 'FAILED'"
        );
        // Pattern: \bpanicked\b
        assert!(
            has_error("thread panicked", Some(0)),
            "Should detect 'panicked'"
        );
    }

    #[test]
    fn test_has_error_with_none_exit_code_and_no_patterns() {
        assert!(!has_error("Output without errors", None));
    }

    #[test]
    fn test_has_error_with_none_exit_code_but_error_pattern() {
        // Pattern: \bcommand not found\b
        assert!(has_error("command not found", None));
        // Pattern: \bsegmentation fault\b
        assert!(has_error("segmentation fault in malloc", None));
    }

    // ─────────────────────────────────────────────────────────────────────
    // is_build_command tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_is_build_command_cargo() {
        assert!(is_build_command("cargo build"));
        assert!(is_build_command("cargo build --release"));
        assert!(is_build_command("cargo check"));
    }

    #[test]
    fn test_is_build_command_npm_and_tsc() {
        assert!(is_build_command("npm run build"));
        assert!(is_build_command("tsc"));
        assert!(is_build_command("make"));
    }

    #[test]
    fn test_is_build_command_non_build() {
        assert!(!is_build_command("ls -la"));
        assert!(!is_build_command("git status"));
        assert!(!is_build_command("echo hello"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // normalize_command tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_normalize_command_multi_word() {
        assert_eq!(normalize_command("cargo build --release"), "cargo build");
        assert_eq!(
            normalize_command("cargo test --lib -- --nocapture"),
            "cargo test"
        );
    }

    #[test]
    fn test_normalize_command_single_word() {
        assert_eq!(normalize_command("ls"), "ls");
        assert_eq!(normalize_command("git"), "git");
    }

    #[test]
    fn test_normalize_command_empty() {
        assert_eq!(normalize_command(""), "");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Importance::as_str tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_importance_as_str() {
        assert_eq!(Importance::Low.as_str(), "low");
        assert_eq!(Importance::Medium.as_str(), "medium");
        assert_eq!(Importance::High.as_str(), "high");
    }

    #[test]
    fn test_temp_state_path_uses_system_temp_dir() {
        let path = temp_state_path("errors", "abc123", "json");
        assert!(path.starts_with(env::temp_dir()));
        assert!(path.ends_with("cortina-errors-abc123.json"));
    }

    #[test]
    fn test_ensure_hyphae_session_with_runner_leaves_state_empty_on_spawn_failure() {
        let hash = "ensure-spawn-failure";
        clear_session_state(hash);

        let state = ensure_hyphae_session_with("demo-project", Some("task"), |_cmd| {
            Err(std::io::Error::other("spawn failed"))
        });

        assert!(state.is_none());
        assert!(load_session_state(hash).is_none());
    }

    #[test]
    fn test_end_hyphae_session_with_missing_state_returns_false() {
        let hash = "end-missing-state";
        clear_session_state(hash);

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Ok(output_with_status(0, ""))
        });

        assert!(!result);
    }

    #[test]
    fn test_end_hyphae_session_with_spawn_failure_keeps_cached_state() {
        let hash = "end-spawn-failure";
        clear_session_state(hash);
        let state = SessionState {
            session_id: "ses_demo".to_string(),
            project: "demo-project".to_string(),
        };
        save_json_file(session_state_path(hash), &state).unwrap();

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Err(std::io::Error::other("spawn failed"))
        });

        assert!(!result);
        assert_eq!(load_session_state(hash).as_ref(), Some(&state));
        clear_session_state(hash);
    }

    #[test]
    fn test_end_hyphae_session_with_non_zero_exit_keeps_cached_state() {
        let hash = "end-non-zero";
        clear_session_state(hash);
        let state = SessionState {
            session_id: "ses_demo".to_string(),
            project: "demo-project".to_string(),
        };
        save_json_file(session_state_path(hash), &state).unwrap();

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Ok(output_with_status(1, "failed"))
        });

        assert!(!result);
        assert_eq!(load_session_state(hash).as_ref(), Some(&state));
        clear_session_state(hash);
    }

    #[test]
    fn test_end_hyphae_session_with_success_clears_cached_state() {
        let hash = "end-success";
        clear_session_state(hash);
        let state = SessionState {
            session_id: "ses_demo".to_string(),
            project: "demo-project".to_string(),
        };
        save_json_file(session_state_path(hash), &state).unwrap();

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Ok(output_with_status(0, "ok"))
        });

        assert!(result);
        assert!(load_session_state(hash).is_none());
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Shared utilities for hook handlers
// ─────────────────────────────────────────────────────────────────────────

use anyhow::Result;
use regex::Regex;
use std::env;
use std::fs;
use std::process::Command;
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

    let cwd =
        env::current_dir().map_or_else(|_| "/tmp".to_string(), |p| p.to_string_lossy().to_string());

    let mut hasher = DefaultHasher::new();
    cwd.hash(&mut hasher);
    format!("{:x}", hasher.finish())
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
pub fn load_json_file<T: serde::de::DeserializeOwned>(path: &str) -> Option<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
}

/// Save JSON to a temp file
pub fn save_json_file<T: serde::Serialize>(path: &str, data: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(data)?;
    fs::write(path, json)?;
    Ok(())
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

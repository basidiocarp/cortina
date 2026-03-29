use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Success,
    Failure,
}

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

pub fn has_error(output: &str, exit_code: Option<i32>) -> bool {
    if let Some(code) = exit_code
        && code != 0
    {
        return true;
    }

    error_patterns().iter().any(|re| re.is_match(output))
}

pub fn is_document_file(path: &str) -> bool {
    let extensions = [
        ".md", ".txt", ".rst", ".adoc", ".json", ".yaml", ".yml", ".toml", ".html", ".css", ".env",
        ".cfg", ".ini", ".sh", ".sql",
    ];

    let path_lower = path.to_lowercase();
    extensions.iter().any(|ext| path_lower.ends_with(ext))
}

pub fn is_build_command(cmd: &str) -> bool {
    build_patterns().iter().any(|re| re.is_match(cmd))
}

pub fn is_test_command(cmd: &str) -> bool {
    test_patterns().iter().any(|re| re.is_match(cmd))
}

pub fn is_significant_command(cmd: &str) -> bool {
    significant_patterns().iter().any(|re| re.is_match(cmd))
}

pub fn successful_validation_feedback(
    cmd: &str,
    exit_code: Option<i32>,
) -> Option<(&'static str, i64, &'static str)> {
    if exit_code != Some(0) {
        return None;
    }

    if is_test_command(cmd) {
        Some(("test_passed", 1, "cortina.post_tool_use.test"))
    } else if is_build_command(cmd) {
        Some(("build_passed", 1, "cortina.post_tool_use.build"))
    } else {
        None
    }
}

pub fn session_outcome_feedback(
    outcome_text: &str,
    saw_failures: bool,
) -> (&'static str, i64, &'static str, SessionOutcome) {
    let normalized = outcome_text.to_ascii_lowercase();
    let failed = saw_failures
        || normalized.contains("failed")
        || normalized.contains("failure")
        || normalized.contains("panic")
        || normalized.contains("aborted");

    if failed {
        (
            "session_failure",
            -1,
            "cortina.stop.session_failure",
            SessionOutcome::Failure,
        )
    } else {
        (
            "session_success",
            1,
            "cortina.stop.session_success",
            SessionOutcome::Success,
        )
    }
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

fn test_patterns() -> &'static [Regex] {
    static TEST_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    TEST_PATTERNS.get_or_init(|| {
        [
            r"\bcargo\s+test\b",
            r"\bnpm\s+test\b",
            r"\bnpm\s+run\s+test\b",
            r"\byarn\s+test\b",
            r"\byarn\s+run\s+test\b",
            r"\bpnpm\s+test\b",
            r"\bpnpm\s+run\s+test\b",
            r"\bbun\s+test\b",
            r"\bpytest\b",
            r"\bgo\s+test\b",
            r"\bvitest\b",
            r"\bjest\b",
            r"\bplaywright\s+test\b",
            r"\bgradlew\s+test\b",
            r"\bmvn\s+test\b",
            r"\bmake\s+test\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
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

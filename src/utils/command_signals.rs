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

    let output = output.to_ascii_lowercase();
    contains_word_followed_by(&output, "error", |ch| matches!(ch, ' ' | ':' | '[' | ']'))
        || contains_word(&output, "failed")
        || contains_word(&output, "failure")
        || contains_word(&output, "panicked")
        || contains_word_followed_by(&output, "fatal", |ch| ch.is_whitespace() || ch == ':')
        || contains_phrase(&output, "command not found")
        || contains_phrase(&output, "segmentation fault")
        || contains_word(&output, "aborted")
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
    let cmd = cmd.to_ascii_lowercase();
    BUILD_PATTERNS
        .iter()
        .any(|pattern| contains_phrase(&cmd, pattern))
}

pub fn is_test_command(cmd: &str) -> bool {
    let cmd = cmd.to_ascii_lowercase();
    TEST_PATTERNS
        .iter()
        .any(|pattern| contains_phrase(&cmd, pattern))
}

pub fn is_significant_command(cmd: &str) -> bool {
    let cmd = cmd.to_ascii_lowercase();
    SIGNIFICANT_PATTERNS
        .iter()
        .any(|pattern| contains_phrase(&cmd, pattern))
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

const BUILD_PATTERNS: &[&str] = &[
    "cargo build",
    "cargo check",
    "npm run build",
    "yarn build",
    "pnpm build",
    "bun build",
    "tsc",
    "next build",
    "make",
    "go build",
    "gradlew build",
    "mvn clean package",
];

const TEST_PATTERNS: &[&str] = &[
    "cargo test",
    "npm test",
    "npm run test",
    "yarn test",
    "yarn run test",
    "pnpm test",
    "pnpm run test",
    "bun test",
    "pytest",
    "go test",
    "vitest",
    "jest",
    "playwright test",
    "gradlew test",
    "mvn test",
    "make test",
];

const SIGNIFICANT_PATTERNS: &[&str] = &[
    "cargo",
    "npm",
    "yarn",
    "pnpm",
    "bun",
    "git push",
    "docker",
    "pytest",
    "make",
    "go build",
    "go test",
    "go run",
    "go vet",
    "rustc",
    "gcc",
    "g++",
    "javac",
    "mvn",
    "gradle",
    "vitest",
    "jest",
    "playwright",
    "tsc",
    "python",
    "ruby",
    "swift",
];

fn contains_phrase(text: &str, phrase: &str) -> bool {
    contains_phrase_with(text, phrase, |_| true)
}

fn contains_word(text: &str, word: &str) -> bool {
    contains_phrase_with(text, word, |_| true)
}

fn contains_word_followed_by(text: &str, word: &str, allow_next: fn(char) -> bool) -> bool {
    contains_phrase_with(text, word, allow_next)
}

fn contains_phrase_with(text: &str, phrase: &str, allow_next: fn(char) -> bool) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(phrase) {
        let index = start + offset;
        let end = index + phrase.len();

        let left_ok = if index == 0 {
            true
        } else {
            text[..index]
                .chars()
                .next_back()
                .is_none_or(|ch| !is_word_char(ch))
        };

        let right_ok = if end == text.len() {
            true
        } else {
            text[end..]
                .chars()
                .next()
                .is_some_and(|ch| allow_next(ch) && !is_word_char(ch))
        };

        if left_ok && right_ok {
            return true;
        }

        start = end;
    }

    false
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

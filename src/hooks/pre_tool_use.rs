use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;

use crate::adapters::claude_code::{ClaudeCodeHookEnvelope, rewrite_response};
use crate::policy::{CapturePolicy, capture_policy};
#[cfg(test)]
use crate::utils::remove_file_with_lock;
use crate::utils::{
    command_exists, resolved_command, scope_hash, temp_state_path, update_json_file,
};

const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp", "rb", "php",
    "swift", "zig", "ex", "exs", "lua", "hs", "cs", "kt", "dart", "vue", "svelte", "astro",
];
const ADVISORY_STATE_NAME: &str = "advisories";
const READ_ADVISORY_MESSAGE: &str = "[cortina] Large code file - consider: \
mcp__rhizome__get_symbols or mcp__rhizome__get_structure for structure, or \
mcp__rhizome__get_symbol_body for a specific function";
const GREP_ADVISORY_MESSAGE: &str = "[cortina] Symbol search - consider: mcp__rhizome__search_symbols or \
mcp__rhizome__find_references";

/// Handle `PreToolUse` adapter events: rewrite commands through Mycelium.
///
/// Replaces mycelium-rewrite.sh. Reads the tool input, checks if the
/// command should be rewritten via `mycelium rewrite`, and outputs the
/// updated input JSON if a rewrite occurred.
#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> Result<()> {
    let envelope = match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            return Ok(());
        }
    };

    if let Some(event) = envelope.command_rewrite_request() {
        if event.command.is_empty() {
            return Ok(());
        }

        // Skip heredocs — they contain too much complexity
        if event.command.contains("<<") {
            return Ok(());
        }

        // ─────────────────────────────────────────────────────────────────────────
        // Check if mycelium is available
        // ─────────────────────────────────────────────────────────────────────────
        if !command_exists("mycelium") {
            return Ok(());
        }

        // ─────────────────────────────────────────────────────────────────────────
        // Delegate to mycelium rewrite
        // ─────────────────────────────────────────────────────────────────────────
        let output = resolved_command("mycelium").map_or_else(
            || {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "mycelium not discoverable",
                ))
            },
            |mut command| command.args(["rewrite", &event.command]).output(),
        );

        let rewritten = match output {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            _ => return Ok(()), // mycelium rewrite failed or exited non-zero, pass through
        };

        // If no change or empty rewrite, nothing to do
        if rewritten.is_empty() || rewritten == event.command {
            return Ok(());
        }

        // ─────────────────────────────────────────────────────────────────────────
        // Output rewrite instruction JSON
        // ─────────────────────────────────────────────────────────────────────────
        let updated_input = event.updated_input_with_command(&rewritten);
        let response = rewrite_response(&updated_input);

        println!("{response}");
        return Ok(());
    }

    if let Some(suggestion) = tool_suggestion_message(&envelope) {
        eprintln!("{suggestion}");
    }

    Ok(())
}

fn tool_suggestion_message(envelope: &ClaudeCodeHookEnvelope) -> Option<String> {
    let policy = capture_policy();
    tool_suggestion_message_with_availability(policy, envelope, command_exists("rhizome"))
}

fn tool_suggestion_message_with_availability(
    policy: &CapturePolicy,
    envelope: &ClaudeCodeHookEnvelope,
    rhizome_available: bool,
) -> Option<String> {
    if !policy.rhizome_suggest_enabled || !rhizome_available {
        return None;
    }

    let advisory = if envelope.tool_name_is("Read") {
        envelope
            .tool_input_string("file_path")
            .and_then(|file_path| {
                read_advisory_for_path(file_path, policy.rhizome_suggest_threshold)
            })
    } else if envelope.tool_name_is("Grep") {
        envelope
            .tool_input_string("pattern")
            .and_then(grep_advisory_for_pattern)
    } else {
        None
    }?;

    advisory_allowed(
        envelope.cwd(),
        &advisory.rate_limit_key,
        policy.rhizome_suggest_every,
    )
    .then_some(advisory.message.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolAdvisory {
    message: &'static str,
    rate_limit_key: String,
}

fn read_advisory_for_path(file_path: &str, threshold: usize) -> Option<ToolAdvisory> {
    if !is_code_file(file_path) {
        return None;
    }

    let line_count = count_lines(file_path).ok()?;
    if line_count <= threshold {
        return None;
    }

    let extension = code_extension(file_path)?;
    Some(ToolAdvisory {
        message: READ_ADVISORY_MESSAGE,
        rate_limit_key: format!("read:{extension}"),
    })
}

fn grep_advisory_for_pattern(pattern: &str) -> Option<ToolAdvisory> {
    let symbol_kind = symbol_like_grep_kind(pattern)?;
    Some(ToolAdvisory {
        message: GREP_ADVISORY_MESSAGE,
        rate_limit_key: format!("grep:{symbol_kind}"),
    })
}

fn is_code_file(file_path: &str) -> bool {
    code_extension(file_path).is_some_and(|extension| CODE_EXTENSIONS.contains(&extension.as_str()))
}

fn code_extension(file_path: &str) -> Option<String> {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

fn count_lines(file_path: &str) -> Result<usize> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    reader.lines().try_fold(0usize, |count, line| {
        line.map(|_| count + 1).map_err(Into::into)
    })
}

fn symbol_like_grep_kind(pattern: &str) -> Option<&'static str> {
    let trimmed = pattern.trim();
    if trimmed.len() < 3 || trimmed.contains(' ') {
        return None;
    }

    let has_trailing_call = trimmed.ends_with('(');
    let identifier = if has_trailing_call {
        trimmed.strip_suffix('(')?
    } else {
        trimmed
    };
    if identifier.is_empty() || !is_ascii_identifier(identifier) {
        return None;
    }

    let invalid_meta = trimmed.char_indices().any(|(index, ch)| {
        regex_meta(ch) && !(has_trailing_call && index == trimmed.len() - 1 && ch == '(')
    });
    if invalid_meta {
        return None;
    }

    if has_trailing_call {
        return Some("call");
    }

    if identifier.chars().next().is_some_and(char::is_uppercase) {
        return Some("type");
    }

    if identifier.contains('_') {
        return Some("symbol");
    }

    let has_upper = identifier.chars().any(char::is_uppercase);
    let has_lower = identifier.chars().any(char::is_lowercase);
    (has_upper && has_lower).then_some("symbol")
}

fn is_ascii_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn regex_meta(ch: char) -> bool {
    matches!(
        ch,
        '.' | '*' | '+' | '?' | '[' | ']' | '{' | '}' | '(' | ')' | '|' | '^' | '$' | '\\'
    )
}

fn advisory_allowed(scope_cwd: Option<&str>, key: &str, cadence: usize) -> bool {
    let hash = scope_hash(scope_cwd);
    let path = temp_state_path(ADVISORY_STATE_NAME, &hash, "json");
    let cadence = cadence.max(1);

    update_json_file::<HashMap<String, usize>, _, _>(&path, |entries| {
        let counter = entries.entry(key.to_string()).or_insert(0);
        let should_emit = *counter % cadence == 0;
        *counter += 1;
        should_emit
    })
    .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn read_advisory_skips_non_code_files() {
        assert!(read_advisory_for_path("README.md", 100).is_none());
        assert!(read_advisory_for_path(".env", 100).is_none());
    }

    #[test]
    fn read_advisory_skips_small_code_files() {
        let temp_dir = std::env::temp_dir().join("cortina-read-suggestion-small");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("small.rs");
        fs::write(&file_path, "fn main() {}\n").unwrap();

        assert!(read_advisory_for_path(file_path.to_str().unwrap(), 100).is_none());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn read_advisory_triggers_for_large_code_files() {
        let temp_dir = std::env::temp_dir().join("cortina-read-suggestion-large");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("large.rs");
        let content = (0..101)
            .map(|i| format!("fn line_{i}() {{}}\n"))
            .collect::<String>();
        fs::write(&file_path, content).unwrap();

        let advisory = read_advisory_for_path(file_path.to_str().unwrap(), 100)
            .expect("large code file should trigger an advisory");
        assert_eq!(advisory.message, READ_ADVISORY_MESSAGE);
        assert_eq!(advisory.rate_limit_key, "read:rs");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn grep_advisory_matches_symbol_like_patterns() {
        assert_eq!(symbol_like_grep_kind("AuthService"), Some("type"));
        assert_eq!(symbol_like_grep_kind("parse_command"), Some("symbol"));
        assert_eq!(symbol_like_grep_kind("handleRequest"), Some("symbol"));
        assert_eq!(symbol_like_grep_kind("parse_command("), Some("call"));

        let advisory =
            grep_advisory_for_pattern("AuthService").expect("symbol search should advise");
        assert_eq!(advisory.message, GREP_ADVISORY_MESSAGE);
        assert_eq!(advisory.rate_limit_key, "grep:type");
    }

    #[test]
    fn grep_advisory_skips_regex_patterns() {
        assert_eq!(symbol_like_grep_kind("foo.*bar"), None);
        assert_eq!(symbol_like_grep_kind("^fn "), None);
        assert_eq!(symbol_like_grep_kind("id"), None);
        assert!(grep_advisory_for_pattern("TODO|FIXME").is_none());
    }

    #[test]
    fn tool_suggestion_requires_rhizome_availability() {
        let envelope = ClaudeCodeHookEnvelope::parse(
            r#"{
                "tool_name": "Grep",
                "tool_input": {"pattern": "AuthService"},
                "cwd": "/tmp/cortina"
            }"#,
        )
        .expect("valid envelope");
        let policy = CapturePolicy::from_reader(|_| None);

        assert!(tool_suggestion_message_with_availability(&policy, &envelope, false).is_none());
    }

    #[test]
    fn advisory_rate_limiting_emits_once_per_cadence() {
        let temp_dir = std::env::temp_dir().join(format!(
            "cortina-advisory-rate-limit-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let scope = temp_dir.to_str().unwrap();
        let hash = scope_hash(Some(scope));
        let path = temp_state_path(ADVISORY_STATE_NAME, &hash, "json");

        assert!(advisory_allowed(Some(scope), "read:rs", 5));
        for _ in 0..4 {
            assert!(!advisory_allowed(Some(scope), "read:rs", 5));
        }
        assert!(advisory_allowed(Some(scope), "read:rs", 5));
        assert!(advisory_allowed(Some(scope), "grep:type", 5));

        let _ = remove_file_with_lock(&path);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn tool_suggestion_respects_rate_limit_per_scope_and_pattern_type() {
        let temp_dir =
            std::env::temp_dir().join(format!("cortina-advisory-scope-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let cwd = temp_dir.to_str().unwrap();
        let hash = scope_hash(Some(cwd));
        let path = temp_state_path(ADVISORY_STATE_NAME, &hash, "json");
        let envelope = ClaudeCodeHookEnvelope::parse(&format!(
            r#"{{
                "tool_name": "Grep",
                "tool_input": {{"pattern": "AuthService"}},
                "cwd": "{cwd}"
            }}"#
        ))
        .expect("valid envelope");
        let policy = CapturePolicy::from_reader(|name| match name {
            "CORTINA_RHIZOME_SUGGEST_EVERY" => Some("2".to_string()),
            _ => None,
        });

        assert!(tool_suggestion_message_with_availability(&policy, &envelope, true).is_some());
        assert!(tool_suggestion_message_with_availability(&policy, &envelope, true).is_none());
        assert!(tool_suggestion_message_with_availability(&policy, &envelope, true).is_some());

        let _ = remove_file_with_lock(&path);
        let _ = fs::remove_dir_all(&temp_dir);
    }
}

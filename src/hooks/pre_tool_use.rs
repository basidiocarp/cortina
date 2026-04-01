use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;

use crate::adapters::claude_code::{ClaudeCodeHookEnvelope, rewrite_response};
use crate::policy::capture_policy;
use crate::utils::{command_exists, resolved_command};

const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp", "rb", "php",
    "swift", "zig", "ex", "exs", "lua", "hs", "cs", "kt", "dart", "vue", "svelte", "astro",
];

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
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
            Err(_) => return Ok(()), // mycelium rewrite failed, pass through
        };

        // If no change, nothing to do
        if rewritten == event.command {
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
    if !policy.rhizome_suggest_enabled || !command_exists("rhizome") {
        return None;
    }

    if envelope.tool_name_is("Read") {
        return envelope
            .tool_input_string("file_path")
            .and_then(|file_path| {
                read_suggestion_for_path(file_path, policy.rhizome_suggest_threshold)
            });
    }

    if envelope.tool_name_is("Grep") {
        return envelope
            .tool_input_string("pattern")
            .and_then(grep_suggestion_for_pattern);
    }

    None
}

fn read_suggestion_for_path(file_path: &str, threshold: usize) -> Option<String> {
    if !is_code_file(file_path) {
        return None;
    }

    let line_count = count_lines(file_path).ok()?;
    if line_count <= threshold {
        return None;
    }

    Some(format!(
        "cortina: rhizome suggestion for {file_path} ({line_count} lines): prefer get_symbols/get_structure/get_symbol_body before full Read"
    ))
}

fn grep_suggestion_for_pattern(pattern: &str) -> Option<String> {
    if !looks_like_symbol(pattern) {
        return None;
    }

    Some(format!(
        "cortina: rhizome suggestion for grep pattern '{pattern}': search_symbols('{pattern}') for semantic matches, find_references for call sites"
    ))
}

fn is_code_file(file_path: &str) -> bool {
    let Some(extension) = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };
    CODE_EXTENSIONS.contains(&extension.as_str())
}

fn count_lines(file_path: &str) -> Result<usize> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    reader.lines().try_fold(0usize, |count, line| {
        line.map(|_| count + 1).map_err(Into::into)
    })
}

fn looks_like_symbol(pattern: &str) -> bool {
    if pattern.len() < 4 {
        return false;
    }
    if pattern.chars().any(|c| ".*+?[]{}()|^$\\".contains(c)) {
        return false;
    }
    if pattern.contains(' ') {
        return false;
    }

    let has_upper = pattern.chars().any(char::is_uppercase);
    let has_lower = pattern.chars().any(char::is_lowercase);
    let has_underscore = pattern.contains('_');

    (has_upper && has_lower) || has_underscore
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn read_suggestion_skips_non_code_files() {
        assert!(read_suggestion_for_path("README.md", 100).is_none());
        assert!(read_suggestion_for_path(".env", 100).is_none());
    }

    #[test]
    fn read_suggestion_skips_small_code_files() {
        let temp_dir = std::env::temp_dir().join("cortina-read-suggestion-small");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("small.rs");
        fs::write(&file_path, "fn main() {}\n").unwrap();

        assert!(read_suggestion_for_path(file_path.to_str().unwrap(), 100).is_none());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn read_suggestion_triggers_for_large_code_files() {
        let temp_dir = std::env::temp_dir().join("cortina-read-suggestion-large");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("large.rs");
        let content = (0..101)
            .map(|i| format!("fn line_{i}() {{}}\n"))
            .collect::<String>();
        fs::write(&file_path, content).unwrap();

        let suggestion = read_suggestion_for_path(file_path.to_str().unwrap(), 100)
            .expect("large code file should trigger a suggestion");
        assert!(suggestion.contains("rhizome suggestion"));
        assert!(suggestion.contains("101 lines"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn grep_symbol_heuristic_prefers_symbol_like_patterns() {
        assert!(looks_like_symbol("AuthService"));
        assert!(looks_like_symbol("validate_token"));
        assert!(grep_suggestion_for_pattern("AuthService").is_some());
    }

    #[test]
    fn grep_symbol_heuristic_skips_regex_and_short_patterns() {
        assert!(!looks_like_symbol("TODO|FIXME"));
        assert!(!looks_like_symbol("fn main"));
        assert!(!looks_like_symbol("id"));
        assert!(grep_suggestion_for_pattern("TODO|FIXME").is_none());
    }
}

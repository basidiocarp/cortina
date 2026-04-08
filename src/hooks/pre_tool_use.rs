use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;
use spore::logging::{SpanContext, subprocess_span, tool_span};
use tracing::{debug, warn};

use crate::adapters::claude_code::{ClaudeCodeHookEnvelope, rewrite_response};
use crate::hooks::pre_commit::handoff_pre_commit_warnings;
use crate::policy::{CapturePolicy, capture_policy};
use crate::utils::{
    command_exists, resolved_command, scope_hash, temp_state_path, update_json_file,
};

#[cfg(test)]
mod tests;

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
            warn!("Failed to parse pre-tool-use adapter input: {e}");
            eprintln!("cortina: failed to parse event input: {e}");
            return Ok(());
        }
    };

    let context = span_context(&envelope);
    let _tool_span = tool_span("pre_tool_use", &context).entered();

    if let Some(event) = envelope.command_rewrite_request() {
        if event.command.is_empty() {
            return Ok(());
        }

        if capture_policy().handoff_lint_enabled {
            for warning in handoff_pre_commit_warnings(&event.command, envelope.cwd()) {
                eprintln!("{warning}");
            }
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
            |mut command| {
                let _subprocess_span = subprocess_span("mycelium rewrite", &context).entered();
                command.args(["rewrite", &event.command]).output()
            },
        );

        let rewritten = match output {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if stderr.trim().is_empty() {
                    debug!("mycelium rewrite exited non-zero; leaving command unchanged");
                } else {
                    warn!("mycelium rewrite exited non-zero: {}", stderr.trim());
                    eprint!("{stderr}");
                }
                return Ok(());
            }
            Err(error) => {
                warn!("Failed to execute mycelium rewrite: {error}");
                eprintln!("cortina: failed to execute mycelium rewrite: {error}");
                return Ok(());
            }
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

fn span_context(envelope: &ClaudeCodeHookEnvelope) -> SpanContext {
    let mut context = SpanContext::for_app("cortina").with_tool("pre_tool_use");
    if let Some(session_id) = envelope.session_id() {
        context = context.with_session_id(session_id.to_string());
    }
    match envelope.cwd() {
        Some(cwd) => context.with_workspace_root(cwd.to_string()),
        None => context,
    }
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
                read_advisory_for_path(file_path, envelope.cwd(), policy.rhizome_suggest_threshold)
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

fn read_advisory_for_path(
    file_path: &str,
    cwd: Option<&str>,
    threshold: usize,
) -> Option<ToolAdvisory> {
    if !is_code_file(file_path) {
        return None;
    }

    let resolved_path = resolve_read_path(file_path, cwd);
    let line_count = count_lines(&resolved_path).ok()?;
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

fn resolve_read_path(file_path: &str, cwd: Option<&str>) -> String {
    let path = Path::new(file_path);
    if path.is_absolute() {
        return file_path.to_string();
    }

    cwd.map(|cwd| Path::new(cwd).join(path).to_string_lossy().into_owned())
        .unwrap_or_else(|| file_path.to_string())
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

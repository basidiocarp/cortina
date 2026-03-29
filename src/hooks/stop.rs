use anyhow::Result;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::events::{OutcomeEvent, OutcomeKind};
use crate::outcomes::{clear_outcomes, load_outcomes};
use crate::utils::{
    Importance, command_exists, end_scoped_hyphae_session, has_error, load_session_state,
    log_hyphae_feedback_signal_for_session, project_name_for_cwd, scope_hash,
    session_outcome_feedback, store_in_hyphae,
};

// ─────────────────────────────────────────────────────────────────────────
// Transcript summary data structure
// ─────────────────────────────────────────────────────────────────────────

struct TranscriptSummary {
    task_desc: String,
    files_modified: Vec<String>,
    tool_counts: String,
    errors_encountered: usize,
    outcome: String,
}

/// Handle Stop adapter events: capture session summary.
///
/// Replaces session-summary.sh. Parses the transcript for task description,
/// files modified, tools used, errors resolved, and outcome.
/// Stores the summary in Hyphae.
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

    let Some(event) = envelope.session_stop_event() else {
        return Ok(());
    };

    if event.cwd.is_empty() {
        return Ok(());
    }

    let hash = scope_hash(Some(&event.cwd));
    let cached_session = load_session_state(&hash);
    let had_cached_session = cached_session.is_some();

    if !command_exists("hyphae") {
        clear_outcomes(&hash);
        return Ok(());
    }

    let project_name = project_name_for_cwd(Some(&event.cwd)).unwrap_or_else(|| {
        Path::new(&event.cwd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    });
    let structured_outcomes = filter_outcomes_for_session(
        &load_outcomes(&hash),
        cached_session.as_ref(),
        &project_name,
    );

    // Parse transcript if available
    let summary = merge_structured_outcomes(
        parse_transcript(event.transcript_path.as_deref()),
        &structured_outcomes,
    );

    // Build summary
    let mut text = format!("Session in {project_name}: {}", summary.task_desc);

    if !summary.files_modified.is_empty() {
        let _ = write!(text, "\nFiles: {}", summary.files_modified.join(", "));
    }

    if !summary.tool_counts.is_empty() {
        let _ = write!(text, "\nTools: {}", summary.tool_counts);
    }

    if summary.errors_encountered > 0 {
        let _ = write!(text, "\nErrors encountered: {}", summary.errors_encountered);
    }

    if !summary.outcome.is_empty() {
        let _ = write!(text, "\nOutcome: {}", summary.outcome);
    }

    if let Some(attribution) = format_structured_outcome_attribution(&structured_outcomes) {
        let _ = write!(text, "\nStructured outcomes: {attribution}");
    }

    let session_feedback = session_outcome_feedback(
        &summary.outcome,
        has_unresolved_errors(&structured_outcomes),
    );

    let ended_structured_session = end_scoped_hyphae_session(
        Some(&event.cwd),
        Some(&text),
        &summary.files_modified,
        summary.errors_encountered,
    );

    if let Some(ref state) = ended_structured_session {
        log_hyphae_feedback_signal_for_session(
            state,
            session_feedback.0,
            session_feedback.1,
            session_feedback.2,
        );
        clear_outcomes(&hash);
    } else if !had_cached_session {
        clear_outcomes(&hash);
    }

    if ended_structured_session.is_none() {
        let topic = format!("session/{project_name}");
        store_in_hyphae(&topic, &text, Importance::Medium, Some(&project_name));
    }
    Ok(())
}

fn merge_structured_outcomes(
    mut summary: TranscriptSummary,
    outcomes: &[OutcomeEvent],
) -> TranscriptSummary {
    if outcomes.is_empty() {
        return summary;
    }

    let mut files: BTreeSet<String> = summary.files_modified.into_iter().collect();
    for outcome in outcomes {
        if let Some(file_path) = outcome.file_path.as_ref().filter(|path| !path.is_empty()) {
            files.insert(file_path.clone());
        }
    }
    summary.files_modified = files.into_iter().collect();

    let structured_error_count = outcomes
        .iter()
        .filter(|event| matches!(event.kind, OutcomeKind::ErrorDetected))
        .count();
    summary.errors_encountered = summary.errors_encountered.max(structured_error_count);

    if (summary.outcome.trim().is_empty() || summary.outcome == "Work completed")
        && let Some(latest) = outcomes.last()
    {
        summary.outcome.clone_from(&latest.summary);
    }

    summary
}

fn format_structured_outcome_attribution(outcomes: &[OutcomeEvent]) -> Option<String> {
    if outcomes.is_empty() {
        return None;
    }

    let mut counts: BTreeMap<OutcomeKind, usize> = BTreeMap::new();
    for outcome in outcomes {
        *counts.entry(outcome.kind).or_insert(0) += 1;
    }

    Some(
        counts
            .into_iter()
            .map(|(kind, count)| format!("{}({count})", kind.label()))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn filter_outcomes_for_session(
    outcomes: &[OutcomeEvent],
    session: Option<&crate::utils::SessionState>,
    project: &str,
) -> Vec<OutcomeEvent> {
    let Some(session) = session else {
        return outcomes.to_vec();
    };

    let session_scoped: Vec<OutcomeEvent> = outcomes
        .iter()
        .filter(|event| event.session_id.as_deref() == Some(session.session_id.as_str()))
        .cloned()
        .collect();

    let unattributed_current_session: Vec<OutcomeEvent> = outcomes
        .iter()
        .filter(|event| {
            event.session_id.is_none()
                && event.project.is_none()
                && event.timestamp >= session.started_at
        })
        .cloned()
        .collect();

    if !session_scoped.is_empty() || !unattributed_current_session.is_empty() {
        return outcomes
            .iter()
            .filter(|event| {
                event.session_id.as_deref() == Some(session.session_id.as_str())
                    || (event.session_id.is_none()
                        && event.project.is_none()
                        && event.timestamp >= session.started_at)
            })
            .cloned()
            .collect();
    }

    if outcomes.iter().any(|event| event.session_id.is_some()) {
        return Vec::new();
    }

    let project_scoped: Vec<OutcomeEvent> = outcomes
        .iter()
        .filter(|event| {
            event.project.as_deref() == Some(project)
                && (session.started_at == 0 || event.timestamp >= session.started_at)
        })
        .cloned()
        .collect();
    if !project_scoped.is_empty() {
        return project_scoped;
    }

    outcomes.to_vec()
}

fn has_unresolved_errors(outcomes: &[OutcomeEvent]) -> bool {
    let detected = outcomes
        .iter()
        .filter(|event| matches!(event.kind, OutcomeKind::ErrorDetected))
        .count();
    let resolved = outcomes
        .iter()
        .filter(|event| matches!(event.kind, OutcomeKind::ErrorResolved))
        .count();

    detected > resolved
}

fn parse_transcript(transcript_path: Option<&str>) -> TranscriptSummary {
    let default = TranscriptSummary {
        task_desc: "Session work".to_string(),
        files_modified: Vec::new(),
        tool_counts: String::new(),
        errors_encountered: 0,
        outcome: "Work completed".to_string(),
    };

    let path = match transcript_path {
        Some(p) if !p.is_empty() => p,
        _ => return default,
    };

    // Try to read and parse transcript JSONL
    match std::fs::read_to_string(path) {
        Ok(content) => parse_jsonl_transcript(&content),
        Err(_) => default,
    }
}

fn parse_jsonl_transcript(content: &str) -> TranscriptSummary {
    let mut summary = TranscriptSummary {
        task_desc: "Session work".to_string(),
        files_modified: Vec::new(),
        tool_counts: String::new(),
        errors_encountered: 0,
        outcome: "Work completed".to_string(),
    };

    let mut tool_usage: HashMap<String, usize> = HashMap::new();
    let mut first_user_message = false;
    let mut file_set = std::collections::HashSet::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            // Extract task description from first user message
            if !first_user_message {
                if let Some("human") = entry.get("type").and_then(|v| v.as_str()) {
                    if let Some(text) = entry.get("text").and_then(|v| v.as_str()) {
                        summary.task_desc = text
                            .replace('\n', " ")
                            .chars()
                            .take(100)
                            .collect::<String>();
                        first_user_message = true;
                    }
                }
            }

            // Extract files from Write/Edit tool calls
            if let Some("tool_use") = entry.get("type").and_then(|v| v.as_str()) {
                if let Some(tool_name) = entry.get("tool_name").and_then(|v| v.as_str()) {
                    // Count tool usage
                    *tool_usage.entry(tool_name.to_string()).or_insert(0) += 1;

                    // Extract file paths from Write/Edit
                    if tool_name == "Write" || tool_name == "Edit" {
                        if let Some(file_path) = entry.get("input").and_then(|v| v.get("file_path"))
                        {
                            if let Some(path_str) = file_path.as_str() {
                                file_set.insert(path_str.to_string());
                            }
                        }
                    }
                }
            }

            // Extract key outcome from last assistant message
            if let Some("assistant") = entry.get("type").and_then(|v| v.as_str()) {
                if let Some(text) = entry.get("text").and_then(|v| v.as_str()) {
                    summary.outcome = text
                        .replace('\n', " ")
                        .chars()
                        .take(150)
                        .collect::<String>();
                }
            }

            // Count errors in tool results
            if let Some("tool_result") = entry.get("type").and_then(|v| v.as_str()) {
                if transcript_tool_result_has_error(&entry) {
                    summary.errors_encountered += 1;
                }
            }
        }
    }

    // Format files modified
    if !file_set.is_empty() {
        let files: Vec<&String> = file_set.iter().collect();
        summary.files_modified = files.into_iter().cloned().collect();
    }

    // Format tool counts
    if !tool_usage.is_empty() {
        let mut counts: Vec<_> = tool_usage.iter().collect();
        counts.sort_by(|a, b| b.1.cmp(a.1)); // Sort by count descending
        let formatted: Vec<String> = counts
            .iter()
            .map(|(tool, count)| format!("{tool}({count})"))
            .collect();
        summary.tool_counts = formatted.join(", ");
    }

    summary
}

fn transcript_tool_result_has_error(entry: &serde_json::Value) -> bool {
    let exit_code = entry
        .get("exit_code")
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let content = entry.get("content").and_then(|value| value.as_str()).unwrap_or("");

    if let Some(code) = exit_code {
        return has_error(content, Some(code));
    }

    let normalized = content.trim();
    normalized.contains("error:")
        || normalized.contains("Error:")
        || normalized.contains("ERROR:")
        || normalized.contains("failed")
        || normalized.contains("Failed")
        || normalized.contains("FAILED")
        || normalized.contains("panic")
        || normalized.contains("Panic")
        || normalized.contains("PANIC")
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{OutcomeEvent, OutcomeKind};

    // ─────────────────────────────────────────────────────────────────────
    // Transcript parsing tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_jsonl_transcript_valid() {
        let jsonl = r#"{"type": "human", "text": "Build and test the project"}
{"type": "tool_use", "tool_name": "Bash", "input": {"command": "cargo build"}}
{"type": "tool_use", "tool_name": "Write", "input": {"file_path": "/path/to/file.rs"}}
{"type": "tool_result", "content": "Build succeeded"}
{"type": "assistant", "text": "Build completed successfully"}
"#;

        let summary = parse_jsonl_transcript(jsonl);

        assert_eq!(summary.task_desc, "Build and test the project");
        assert!(!summary.outcome.is_empty());
        assert!(summary.outcome.contains("successfully"));
    }

    #[test]
    fn test_parse_jsonl_transcript_counts_tools() {
        let jsonl = r#"{"type": "human", "text": "Task description"}
{"type": "tool_use", "tool_name": "Bash", "input": {}}
{"type": "tool_use", "tool_name": "Bash", "input": {}}
{"type": "tool_use", "tool_name": "Edit", "input": {"file_path": "/test.rs"}}
{"type": "assistant", "text": "Done"}
"#;

        let summary = parse_jsonl_transcript(jsonl);

        assert!(summary.tool_counts.contains("Bash"));
        assert!(summary.tool_counts.contains("Edit"));
    }

    #[test]
    fn test_parse_jsonl_transcript_counts_errors() {
        let jsonl = r#"{"type": "human", "text": "Task"}
{"type": "tool_use", "tool_name": "Bash", "input": {}}
{"type": "tool_result", "content": "error: failed"}
{"type": "tool_result", "content": "FAILED: compilation"}
{"type": "assistant", "text": "Done"}
"#;

        let summary = parse_jsonl_transcript(jsonl);

        assert_eq!(summary.errors_encountered, 2);
    }

    #[test]
    fn test_parse_jsonl_transcript_ignores_error_handling_prose() {
        let jsonl = r#"{"type": "human", "text": "Task"}
{"type": "tool_result", "content": "Improved error handling and validation flow"}
{"type": "assistant", "text": "Done"}
"#;

        let summary = parse_jsonl_transcript(jsonl);

        assert_eq!(summary.errors_encountered, 0);
    }

    #[test]
    fn test_parse_jsonl_transcript_empty_input() {
        let empty = "";

        let summary = parse_jsonl_transcript(empty);

        assert_eq!(summary.task_desc, "Session work");
        assert_eq!(summary.outcome, "Work completed");
        assert_eq!(summary.errors_encountered, 0);
        assert!(summary.files_modified.is_empty());
    }

    #[test]
    fn test_parse_jsonl_transcript_extracts_files() {
        let jsonl = r#"{"type": "human", "text": "Modify files"}
{"type": "tool_use", "tool_name": "Write", "input": {"file_path": "/a.rs"}}
{"type": "tool_use", "tool_name": "Edit", "input": {"file_path": "/b.rs"}}
{"type": "assistant", "text": "Done"}
"#;

        let summary = parse_jsonl_transcript(jsonl);

        assert!(summary.files_modified.iter().any(|file| file == "/a.rs"));
        assert!(summary.files_modified.iter().any(|file| file == "/b.rs"));
    }

    #[test]
    fn test_merge_structured_outcomes_enriches_summary() {
        let summary = TranscriptSummary {
            task_desc: "Session work".to_string(),
            files_modified: vec!["/tmp/a.rs".to_string()],
            tool_counts: String::new(),
            errors_encountered: 0,
            outcome: "Work completed".to_string(),
        };
        let outcomes = vec![
            OutcomeEvent::new(OutcomeKind::ErrorDetected, "Command failed: cargo test")
                .with_command("cargo test"),
            OutcomeEvent::new(
                OutcomeKind::SelfCorrection,
                "Corrected recent edit in /tmp/b.rs",
            )
            .with_file_path("/tmp/b.rs"),
        ];

        let merged = merge_structured_outcomes(summary, &outcomes);

        assert_eq!(merged.errors_encountered, 1);
        assert_eq!(merged.outcome, "Corrected recent edit in /tmp/b.rs");
        assert!(merged.files_modified.iter().any(|path| path == "/tmp/a.rs"));
        assert!(merged.files_modified.iter().any(|path| path == "/tmp/b.rs"));
    }

    #[test]
    fn test_format_structured_outcome_attribution_counts_by_kind() {
        let outcomes = vec![
            OutcomeEvent::new(OutcomeKind::ErrorDetected, "first"),
            OutcomeEvent::new(OutcomeKind::ErrorDetected, "second"),
            OutcomeEvent::new(OutcomeKind::ValidationPassed, "cargo test passed"),
        ];

        let formatted = format_structured_outcome_attribution(&outcomes).unwrap();
        assert!(formatted.contains("error_detected(2)"));
        assert!(formatted.contains("validation_passed(1)"));
    }

    #[test]
    fn test_filter_outcomes_for_session_prefers_matching_session_id() {
        let session = crate::utils::SessionState {
            session_id: "ses_current".to_string(),
            project: "demo".to_string(),
            started_at: 100,
        };
        let outcomes = vec![
            OutcomeEvent::new(OutcomeKind::ErrorDetected, "old").with_session("ses_old", "demo"),
            OutcomeEvent::new(OutcomeKind::ValidationPassed, "current")
                .with_session("ses_current", "demo"),
        ];

        let filtered = filter_outcomes_for_session(&outcomes, Some(&session), "demo");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].summary, "current");
    }

    #[test]
    fn test_filter_outcomes_for_session_keeps_current_unattributed_outcomes_only() {
        let session = crate::utils::SessionState {
            session_id: "ses_current".to_string(),
            project: "demo".to_string(),
            started_at: 200,
        };
        let mut old = OutcomeEvent::new(OutcomeKind::ErrorDetected, "old unattributed");
        old.timestamp = 150;
        let mut current = OutcomeEvent::new(OutcomeKind::ValidationPassed, "current unattributed");
        current.timestamp = 250;
        let outcomes = vec![old, current];

        let filtered = filter_outcomes_for_session(&outcomes, Some(&session), "demo");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].summary, "current unattributed");
    }

    #[test]
    fn test_has_unresolved_errors_stays_true_after_non_error_outcome() {
        let outcomes = vec![
            OutcomeEvent::new(OutcomeKind::ErrorDetected, "command failed"),
            OutcomeEvent::new(OutcomeKind::DocumentIngested, "docs ingested"),
        ];

        assert!(has_unresolved_errors(&outcomes));
    }
}

use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use crate::utils::{Importance, command_exists, store_in_hyphae};

// ─────────────────────────────────────────────────────────────────────────
// Transcript summary data structure
// ─────────────────────────────────────────────────────────────────────────

struct TranscriptSummary {
    task_desc: String,
    files_modified: String,
    tool_counts: String,
    errors_encountered: usize,
    outcome: String,
}

/// Handle Stop events: capture session summary.
///
/// Replaces session-summary.sh. Parses the transcript for task description,
/// files modified, tools used, errors resolved, and outcome.
/// Stores the summary in Hyphae.
#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> Result<()> {
    let json: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("cortina: failed to parse hook input: {e}");
            return Ok(());
        }
    };

    let transcript_path = json.get("transcript_path").and_then(|v| v.as_str());
    let cwd = json.get("cwd").and_then(|v| v.as_str());

    // Need at least cwd to be useful
    let cwd = match cwd {
        Some(c) if !c.is_empty() => c,
        _ => return Ok(()),
    };

    if !command_exists("hyphae") {
        return Ok(());
    }

    let project_name = Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Parse transcript if available
    let summary = parse_transcript(transcript_path);

    // Build summary
    let mut text = format!("Session in {project_name}: {}", summary.task_desc);

    if !summary.files_modified.is_empty() {
        let _ = write!(text, "\nFiles: {}", summary.files_modified);
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

    // Store in Hyphae (fire and forget with timeout)
    let topic = format!("session/{project_name}");
    store_in_hyphae(&topic, &text, Importance::Medium, Some(project_name));

    Ok(())
}

fn parse_transcript(transcript_path: Option<&str>) -> TranscriptSummary {
    let default = TranscriptSummary {
        task_desc: "Session work".to_string(),
        files_modified: String::new(),
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
        files_modified: String::new(),
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
                if let Some(content_str) = entry.get("content").and_then(|v| v.as_str()) {
                    if content_str.contains("error")
                        || content_str.contains("Error")
                        || content_str.contains("ERROR")
                        || content_str.contains("failed")
                        || content_str.contains("Failed")
                        || content_str.contains("FAILED")
                        || content_str.contains("panic")
                    {
                        summary.errors_encountered += 1;
                    }
                }
            }
        }
    }

    // Format files modified
    if !file_set.is_empty() {
        let files: Vec<&String> = file_set.iter().collect();
        summary.files_modified = files
            .iter()
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(", ");
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

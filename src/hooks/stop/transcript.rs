use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};

use serde_json::Value;

use crate::utils::has_error;

use super::summary::TranscriptSummary;

pub(super) fn parse_transcript(transcript_path: Option<&str>) -> TranscriptSummary {
    let path = match transcript_path {
        Some(p) if !p.is_empty() => p,
        _ => return TranscriptSummary::default(),
    };

    match std::fs::File::open(path) {
        Ok(file) => parse_jsonl_transcript_streaming(BufReader::new(file)),
        Err(e) => {
            tracing::warn!(path = ?path, error = %e, "cortina: transcript parse failed");
            TranscriptSummary::default()
        }
    }
}

/// Parse a JSONL transcript from a `&str` (used in tests).
///
/// In production, `parse_transcript` uses the streaming `BufReader` path to
/// avoid loading the full file into memory. This wrapper lets existing tests
/// continue to call a `&str`-accepting signature without modification.
#[cfg(test)]
pub(super) fn parse_jsonl_transcript(content: &str) -> TranscriptSummary {
    parse_jsonl_transcript_streaming(std::io::Cursor::new(content))
}

/// Parse a JSONL transcript from a `BufRead` source, extracting only what
/// cortina needs (first human message, tool usage counts, file paths, errors).
/// Using a streaming reader avoids loading the full transcript into memory —
/// long sessions can produce 100+ MB transcripts.
fn parse_jsonl_transcript_streaming(reader: impl BufRead) -> TranscriptSummary {
    let mut summary = TranscriptSummary::default();
    let mut tool_usage: HashMap<String, usize> = HashMap::new();
    let mut first_user_message = false;
    let mut file_set = HashSet::new();

    for line_result in reader.lines() {
        let raw = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            // Human text: entry["message"]["content"][0]["text"]
            if !first_user_message
                && let Some("human") = entry.get("type").and_then(Value::as_str)
                && let Some(text) = entry
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|el| el.get("text"))
                    .and_then(Value::as_str)
            {
                summary.task_desc = text
                    .replace('\n', " ")
                    .chars()
                    .take(100)
                    .collect::<String>();
                first_user_message = true;
            }

            // Tool use: walk entry["message"]["content"] for type == "tool_use"
            if let Some("tool_use") = entry.get("type").and_then(Value::as_str) {
                if let Some(content_arr) = entry
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for el in content_arr {
                        if el.get("type").and_then(Value::as_str) == Some("tool_use") {
                            if let Some(tool_name) = el.get("name").and_then(Value::as_str) {
                                *tool_usage.entry(tool_name.to_string()).or_insert(0) += 1;

                                if (tool_name == "Write" || tool_name == "Edit")
                                    && let Some(path_str) = el
                                        .get("input")
                                        .and_then(|v| v.get("file_path"))
                                        .and_then(Value::as_str)
                                {
                                    file_set.insert(path_str.to_string());
                                }
                            }
                        }
                    }
                }
            }

            // Assistant text: walk entry["message"]["content"] for type == "text"
            if let Some("assistant") = entry.get("type").and_then(Value::as_str)
                && let Some(content_arr) = entry
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
            {
                for el in content_arr {
                    if el.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(text) = el.get("text").and_then(Value::as_str) {
                            summary.outcome = text
                                .replace('\n', " ")
                                .chars()
                                .take(150)
                                .collect::<String>();
                            break;
                        }
                    }
                }
            }

            if let Some("tool_result") = entry.get("type").and_then(Value::as_str)
                && transcript_tool_result_has_error(&entry)
            {
                summary.errors_encountered += 1;
            }
        }
    }

    if !file_set.is_empty() {
        let files: Vec<&String> = file_set.iter().collect();
        summary.files_modified = files.into_iter().cloned().collect();
    }

    if !tool_usage.is_empty() {
        let mut counts: Vec<_> = tool_usage.iter().collect();
        counts.sort_by(|a, b| b.1.cmp(a.1));
        let formatted: Vec<String> = counts
            .iter()
            .map(|(tool, count)| format!("{tool}({count})"))
            .collect();
        summary.tool_counts = formatted.join(", ");
    }

    summary
}

/// Exit codes that are non-zero but non-fatal for common tools:
///
/// - 1: grep (no match), diff (differences found), many POSIX utilities
///
/// These produce false-positive error counts that misclassify valid sessions.
const NON_FATAL_EXIT_CODES: &[i32] = &[1];

fn transcript_tool_result_has_error(entry: &Value) -> bool {
    let exit_code = entry
        .get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let content = entry.get("content").and_then(Value::as_str).unwrap_or("");

    if let Some(code) = exit_code {
        // Skip non-fatal exit codes that tools like grep and diff use for
        // "no match" / "differences found" — not errors from our perspective.
        if NON_FATAL_EXIT_CODES.contains(&code) {
            return false;
        }
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

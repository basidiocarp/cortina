use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::utils::has_error;

use super::summary::TranscriptSummary;

pub(super) fn parse_transcript(transcript_path: Option<&str>) -> TranscriptSummary {
    let path = match transcript_path {
        Some(p) if !p.is_empty() => p,
        _ => return TranscriptSummary::default(),
    };

    match std::fs::read_to_string(path) {
        Ok(content) => parse_jsonl_transcript(&content),
        Err(_) => TranscriptSummary::default(),
    }
}

pub(super) fn parse_jsonl_transcript(content: &str) -> TranscriptSummary {
    let mut summary = TranscriptSummary::default();
    let mut tool_usage: HashMap<String, usize> = HashMap::new();
    let mut first_user_message = false;
    let mut file_set = HashSet::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            if !first_user_message
                && let Some("human") = entry.get("type").and_then(Value::as_str)
                && let Some(text) = entry.get("text").and_then(Value::as_str)
            {
                summary.task_desc = text
                    .replace('\n', " ")
                    .chars()
                    .take(100)
                    .collect::<String>();
                first_user_message = true;
            }

            if let Some("tool_use") = entry.get("type").and_then(Value::as_str)
                && let Some(tool_name) = entry.get("tool_name").and_then(Value::as_str)
            {
                *tool_usage.entry(tool_name.to_string()).or_insert(0) += 1;

                if (tool_name == "Write" || tool_name == "Edit")
                    && let Some(path_str) = entry
                        .get("input")
                        .and_then(|v| v.get("file_path"))
                        .and_then(Value::as_str)
                {
                    file_set.insert(path_str.to_string());
                }
            }

            if let Some("assistant") = entry.get("type").and_then(Value::as_str)
                && let Some(text) = entry.get("text").and_then(Value::as_str)
            {
                summary.outcome = text
                    .replace('\n', " ")
                    .chars()
                    .take(150)
                    .collect::<String>();
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

fn transcript_tool_result_has_error(entry: &Value) -> bool {
    let exit_code = entry
        .get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let content = entry.get("content").and_then(Value::as_str).unwrap_or("");

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

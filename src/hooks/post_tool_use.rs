use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::event_envelope::{BashToolEvent, EventEnvelope, FileEditEvent, PostToolUseEvent};
use crate::utils::{
    Importance, command_exists, cwd_hash, get_project_name, has_error, is_build_command,
    is_document_file, is_significant_command, load_json_file, normalize_command, save_json_file,
    spawn_async, store_in_hyphae, temp_state_path,
};

const CORRECTION_WINDOW_MS: u64 = 5 * 60 * 1000; // 5 minutes
const CLEANUP_AGE_MS: u64 = 10 * 60 * 1000; // 10 minutes
const EXPORT_THRESHOLD: usize = 5;
const INGEST_THRESHOLD: usize = 3;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ErrorEntry {
    command: String,
    error: String,
    timestamp: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct EditEntry {
    file: String,
    old_string: String,
    new_string: String,
    timestamp: u64,
}

/// Handle `PostToolUse` events: capture errors, corrections, code changes.
///
/// Replaces capture-errors.js, capture-corrections.js, capture-code-changes.js.
/// Reads the tool result, detects patterns, stores signals in Hyphae.
#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> Result<()> {
    let envelope = match EventEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            return Ok(());
        }
    };

    match envelope.post_tool_use_event() {
        Some(PostToolUseEvent::Bash(event)) => handle_bash(&event),
        Some(PostToolUseEvent::FileEdit(event)) => handle_file_edits(&event),
        None => {}
    }

    // Pass through the original input (hooks must not modify output)
    print!("{input}");
    Ok(())
}

/// Handle Bash tool calls: detect errors and resolutions
fn handle_bash(event: &BashToolEvent) {
    let command = event.command.as_str();
    let output = event.output.as_str();
    let exit_code = event.exit_code;

    if command.is_empty() || !is_significant_command(command) {
        return;
    }

    let hash = cwd_hash();
    let track_file = temp_state_path("errors", &hash, "json");
    let cmd_key = normalize_command(command);
    let error_detected = has_error(output, exit_code);

    if error_detected {
        track_error(&track_file, &cmd_key, command, output);
        if command_exists("hyphae") {
            store_error_in_hyphae(command, output);
        }
    } else {
        resolve_error(&track_file, &cmd_key, command);
    }

    // Check for build success and trigger exports
    if is_build_command(command) && exit_code.is_none_or(|c| c == 0) {
        check_and_trigger_exports(&hash);
    }
}

/// Handle Write/Edit/MultiEdit: track edits and detect corrections
fn handle_file_edits(event: &FileEditEvent) {
    let file_path = event.file_path.as_str();
    let old_str = event.old_string.as_str();
    let new_str = event.new_string.as_str();

    if file_path.is_empty() || old_str.is_empty() {
        return;
    }

    let hash = cwd_hash();
    let track_file = temp_state_path("edits", &hash, "json");

    // Check for self-corrections
    if let Some(prev_edit) = find_correction(file_path, old_str, &track_file) {
        if command_exists("hyphae") {
            store_correction_in_hyphae(file_path, &prev_edit, old_str, new_str);
        }
    }

    // Track this edit
    track_edit(&track_file, file_path, old_str, new_str);

    // Track file edits for rhizome export
    track_pending_file(file_path, &hash);

    // Track document edits for hyphae ingest
    if is_document_file(file_path) {
        track_pending_document(file_path, &hash);
    }

    // Check thresholds and trigger exports
    let pending_docs = get_pending_documents(&hash);
    if pending_docs.len() >= INGEST_THRESHOLD && command_exists("hyphae") {
        trigger_hyphae_ingest(&pending_docs);
        clear_pending_documents(&hash);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Error tracking
// ─────────────────────────────────────────────────────────────────────────

fn track_error(track_file: &Path, cmd_key: &str, command: &str, output: &str) {
    let mut entries: HashMap<String, ErrorEntry> = load_json_file(track_file).unwrap_or_default();
    entries.insert(
        cmd_key.to_string(),
        ErrorEntry {
            command: command.chars().take(500).collect(),
            error: output.chars().take(500).collect(),
            timestamp: u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            )
            .unwrap_or(u64::MAX),
        },
    );
    let _ = save_json_file(track_file, &entries);
}

fn resolve_error(track_file: &Path, cmd_key: &str, command: &str) {
    let mut entries: HashMap<String, ErrorEntry> = load_json_file(track_file).unwrap_or_default();

    if let Some(prev_error) = entries.remove(cmd_key) {
        let _ = save_json_file(track_file, &entries);

        if command_exists("hyphae") {
            let content = format!(
                "Fixed: {}\nPrevious error: {}",
                &command[..command.len().min(200)],
                &prev_error.error[..prev_error.error.len().min(300)]
            );
            store_in_hyphae(
                "errors/resolved",
                &content,
                Importance::High,
                get_project_name().as_deref(),
            );
        }
    }
}

fn store_error_in_hyphae(command: &str, output: &str) {
    let content = format!(
        "Command: {}\nError: {}",
        &command[..command.len().min(200)],
        &output[..output.len().min(500)]
    );
    store_in_hyphae(
        "errors/active",
        &content,
        Importance::Medium,
        get_project_name().as_deref(),
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Edit tracking and corrections
// ─────────────────────────────────────────────────────────────────────────

fn load_and_clean_edits(track_file: &Path) -> Vec<EditEntry> {
    let mut edits: Vec<EditEntry> = load_json_file(track_file).unwrap_or_default();

    let cutoff = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
    .saturating_sub(CLEANUP_AGE_MS);

    edits.retain(|e| e.timestamp > cutoff);
    edits
}

fn track_edit(track_file: &Path, file_path: &str, old_str: &str, new_str: &str) {
    let mut edits = load_and_clean_edits(track_file);
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX);

    edits.push(EditEntry {
        file: file_path.to_string(),
        old_string: old_str.chars().take(200).collect(),
        new_string: new_str.chars().take(200).collect(),
        timestamp: now,
    });

    let _ = save_json_file(track_file, &edits);
}

fn find_correction(file_path: &str, old_str: &str, track_file: &Path) -> Option<EditEntry> {
    let edits = load_and_clean_edits(track_file);
    let cutoff = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
    .saturating_sub(CORRECTION_WINDOW_MS);

    let candidates: Vec<_> = edits
        .iter()
        .filter(|e| e.file == file_path && e.timestamp > cutoff && !e.new_string.is_empty())
        .collect();

    for prev in candidates {
        // Correction: current old_string overlaps previous new_string
        if prev.new_string.contains(old_str) || old_str.contains(&prev.new_string) {
            return Some(prev.clone());
        }
    }

    None
}

fn store_correction_in_hyphae(
    file_path: &str,
    corrected_edit: &EditEntry,
    new_old_str: &str,
    new_new_str: &str,
) {
    let file_name = Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file_path);

    let content = format!(
        "File: {}\nOriginal change: {} → {}\nCorrection: {} → {}",
        file_name,
        corrected_edit.old_string,
        corrected_edit.new_string,
        &new_old_str[..new_old_str.len().min(200)],
        &new_new_str[..new_new_str.len().min(200)]
    );

    store_in_hyphae(
        "corrections",
        &content,
        Importance::High,
        get_project_name().as_deref(),
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Rhizome export and Hyphae ingest tracking
// ─────────────────────────────────────────────────────────────────────────

fn get_pending_files_path(hash: &str) -> std::path::PathBuf {
    temp_state_path("pending-exports", hash, "txt")
}

fn get_pending_documents_path(hash: &str) -> std::path::PathBuf {
    temp_state_path("pending-ingest", hash, "txt")
}

fn get_pending_files(hash: &str) -> Vec<String> {
    let path = get_pending_files_path(hash);
    std::fs::read_to_string(&path)
        .ok()
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn get_pending_documents(hash: &str) -> Vec<String> {
    let path = get_pending_documents_path(hash);
    std::fs::read_to_string(&path)
        .ok()
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn track_pending_file(file_path: &str, hash: &str) {
    let path = get_pending_files_path(hash);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{file_path}")
        });
}

fn track_pending_document(file_path: &str, hash: &str) {
    let path = get_pending_documents_path(hash);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{file_path}")
        });
}

fn clear_pending_files(hash: &str) {
    let path = get_pending_files_path(hash);
    let _ = std::fs::remove_file(&path);
}

fn clear_pending_documents(hash: &str) {
    let path = get_pending_documents_path(hash);
    let _ = std::fs::remove_file(&path);
}

fn check_and_trigger_exports(hash: &str) {
    let pending_files = get_pending_files(hash);
    if pending_files.len() >= EXPORT_THRESHOLD && command_exists("rhizome") {
        spawn_async("rhizome", &["export"]);
        clear_pending_files(hash);
    }
}

fn trigger_hyphae_ingest(documents: &[String]) {
    for doc in documents {
        spawn_async("hyphae", &["ingest-file", doc]);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─────────────────────────────────────────────────────────────────────
    // Bash hook event JSON parsing tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_bash_hook_event_valid() {
        let json_str = r#"{
            "tool_name": "Bash",
            "tool_input": {
                "command": "cargo test --lib"
            },
            "tool_output": {
                "output": "test output",
                "exit_code": 0
            }
        }"#;

        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let tool_name = json.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(tool_name, "Bash");

        let command = json
            .get("tool_input")
            .and_then(|v| v.get("command"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(command, "cargo test --lib");

        let exit_code: Option<i32> = json
            .get("tool_output")
            .and_then(|v| v.get("exit_code"))
            .and_then(serde_json::Value::as_i64)
            .and_then(|i| i32::try_from(i).ok());
        assert_eq!(exit_code, Some(0));
    }

    #[test]
    fn test_parse_bash_hook_with_error_exit_code() {
        let json_str = r#"{
            "tool_name": "Bash",
            "tool_input": {
                "command": "cargo build"
            },
            "tool_output": {
                "output": "error: failed to compile",
                "exit_code": 101
            }
        }"#;

        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let exit_code: Option<i32> = json
            .get("tool_output")
            .and_then(|v| v.get("exit_code"))
            .and_then(serde_json::Value::as_i64)
            .and_then(|i| i32::try_from(i).ok());
        assert_eq!(exit_code, Some(101));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Edit hook event JSON parsing tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_edit_hook_event_valid() {
        let json_str = r#"{
            "tool_name": "Edit",
            "tool_input": {
                "file_path": "/path/to/file.rs",
                "old_string": "fn old() {}",
                "new_string": "fn new() {}"
            }
        }"#;

        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let tool_name = json.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(tool_name, "Edit");

        let file_path = json
            .get("tool_input")
            .and_then(|v| v.get("file_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(file_path, "/path/to/file.rs");

        let old_string = json
            .get("tool_input")
            .and_then(|v| v.get("old_string"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(old_string, "fn old() {}");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Malformed JSON handling tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_handle_malformed_json_returns_ok() {
        let malformed = r#"{ invalid json }"#;
        let result = handle(malformed);
        // Should not panic, should return Ok
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_empty_json_returns_ok() {
        let empty = "";
        let result = handle(empty);
        // Should handle gracefully
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_valid_json_with_unknown_tool() {
        let json_str = r#"{
            "tool_name": "UnknownTool",
            "tool_input": {}
        }"#;

        let result = handle(json_str);
        assert!(result.is_ok());
    }
}

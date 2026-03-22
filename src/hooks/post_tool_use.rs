use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::utils::{
    Importance, command_exists, cwd_hash, get_project_name, has_error, is_build_command,
    is_document_file, is_significant_command, load_json_file, normalize_command, save_json_file,
    spawn_async, store_in_hyphae,
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
    let json: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("cortina: failed to parse hook input: {e}");
            return Ok(());
        }
    };

    let tool_name = json.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");

    match tool_name {
        "Bash" => {
            handle_bash(&json);
        }
        "Write" | "Edit" | "MultiEdit" => {
            handle_file_edits(&json);
        }
        _ => {}
    }

    // Pass through the original input (hooks must not modify output)
    print!("{input}");
    Ok(())
}

/// Handle Bash tool calls: detect errors and resolutions
fn handle_bash(json: &serde_json::Value) {
    let command = json
        .get("tool_input")
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let output = json
        .get("tool_output")
        .and_then(|v| v.get("output"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let exit_code: Option<i32> = json
        .get("tool_output")
        .and_then(|v| v.get("exit_code"))
        .and_then(serde_json::Value::as_i64)
        .and_then(|i| i32::try_from(i).ok());

    if command.is_empty() || !is_significant_command(command) {
        return;
    }

    let hash = cwd_hash();
    let track_file = format!("/tmp/cortina-errors-{hash}.json");
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
fn handle_file_edits(json: &serde_json::Value) {
    let file_path = json
        .get("tool_input")
        .and_then(|v| v.get("file_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let old_str = json
        .get("tool_input")
        .and_then(|v| v.get("old_string"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let new_str = json
        .get("tool_input")
        .and_then(|v| v.get("new_string"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if file_path.is_empty() || old_str.is_empty() {
        return;
    }

    let hash = cwd_hash();
    let track_file = format!("/tmp/cortina-edits-{hash}.json");

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

fn track_error(track_file: &str, cmd_key: &str, command: &str, output: &str) {
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

fn resolve_error(track_file: &str, cmd_key: &str, command: &str) {
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

fn load_and_clean_edits(track_file: &str) -> Vec<EditEntry> {
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

fn track_edit(track_file: &str, file_path: &str, old_str: &str, new_str: &str) {
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

fn find_correction(file_path: &str, old_str: &str, track_file: &str) -> Option<EditEntry> {
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

fn get_pending_files_path(hash: &str) -> String {
    format!("/tmp/cortina-pending-exports-{hash}.txt")
}

fn get_pending_documents_path(hash: &str) -> String {
    format!("/tmp/cortina-pending-ingest-{hash}.txt")
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

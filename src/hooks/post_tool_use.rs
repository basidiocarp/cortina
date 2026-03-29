use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::events::{BashToolEvent, FileEditEvent, OutcomeEvent, OutcomeKind, ToolResultEvent};
use crate::outcomes::record_outcome;
use crate::utils::{
    Importance, SessionState, command_exists, current_timestamp_ms,
    ensure_scoped_hyphae_session, has_error, is_build_command, is_document_file,
    is_significant_command, load_json_file, log_scoped_hyphae_feedback_signal, normalize_command,
    project_name_for_cwd, scope_hash, spawn_async_checked, store_in_hyphae,
    successful_validation_feedback, temp_state_path, update_json_file,
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

/// Handle `PostToolUse` adapter events: capture errors, corrections, code changes.
///
/// Replaces capture-errors.js, capture-corrections.js, capture-code-changes.js.
/// Reads the tool result, detects patterns, stores signals in Hyphae.
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

    match envelope.tool_result_event() {
        Some(ToolResultEvent::Bash(event)) => handle_bash(&event),
        Some(ToolResultEvent::FileEdit(event)) => handle_file_edits(&event),
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
    let scope_cwd = event.cwd.as_deref();

    if command.is_empty() || !is_significant_command(command) {
        return;
    }

    if command_exists("hyphae") {
        let _ = ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200)));
    }

    let hash = scope_hash(scope_cwd);
    let track_file = temp_state_path("errors", &hash, "json");
    let cmd_key = normalize_command(command);
    let error_detected = has_error(output, exit_code);

    if error_detected {
        track_error(&track_file, &cmd_key, command, output);
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200))),
            OutcomeEvent::new(
                OutcomeKind::ErrorDetected,
                format!("Command failed: {}", truncate(command, 200)),
            )
            .with_command(truncate(command, 500)),
        );
        record_outcome(&hash, outcome);
        if command_exists("hyphae") {
            store_error_in_hyphae(command, output, scope_cwd);
            log_scoped_hyphae_feedback_signal(
                scope_cwd,
                "tool_error",
                -1,
                "cortina.post_tool_use.error_detected",
                Some(&truncate(command, 200)),
            );
        }
    } else {
        resolve_error(&track_file, &cmd_key, command, &hash, scope_cwd);
        log_validation_success(command, exit_code, &hash, scope_cwd);
    }

    // Check for build success and trigger exports
    if is_build_command(command) && exit_code.is_none_or(|c| c == 0) {
        check_and_trigger_exports(&hash, scope_cwd);
    }
}

/// Handle Write/Edit/MultiEdit: track edits and detect corrections
fn handle_file_edits(event: &FileEditEvent) {
    let file_path = event.file_path.as_str();
    let old_str = event.old_string.as_str();
    let new_str = event.new_string.as_str();
    let scope_cwd = event.cwd.as_deref();

    if file_path.is_empty() {
        return;
    }

    let hash = scope_hash(scope_cwd);
    let track_file = temp_state_path("edits", &hash, "json");

    // Check for self-corrections
    if !old_str.is_empty()
        && let Some(prev_edit) = find_correction(file_path, old_str, &track_file)
    {
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(file_path, 200))),
            OutcomeEvent::new(
                OutcomeKind::SelfCorrection,
                format!("Corrected recent edit in {}", truncate(file_path, 200)),
            )
            .with_file_path(truncate(file_path, 500)),
        );
        record_outcome(&hash, outcome);
        if command_exists("hyphae") {
            store_correction_in_hyphae(file_path, &prev_edit, old_str, new_str, scope_cwd);
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
    if command_exists("hyphae") {
        let pending_docs = take_pending_documents_batch(&hash);
        if !pending_docs.is_empty() {
            let failed_docs = trigger_hyphae_ingest(&pending_docs, &hash, scope_cwd);
            if !failed_docs.is_empty() {
                requeue_pending_documents(&failed_docs, &hash);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Error tracking
// ─────────────────────────────────────────────────────────────────────────

fn track_error(track_file: &Path, cmd_key: &str, command: &str, output: &str) {
    let _ = update_json_file::<HashMap<String, ErrorEntry>, _, _>(track_file, |entries| {
        entries.insert(
            cmd_key.to_string(),
            ErrorEntry {
                command: command.chars().take(500).collect(),
                error: output.chars().take(500).collect(),
                timestamp: current_timestamp_ms(),
            },
        );
    });
}

fn resolve_error(
    track_file: &Path,
    cmd_key: &str,
    command: &str,
    hash: &str,
    scope_cwd: Option<&str>,
) {
    let prev_error = update_json_file::<HashMap<String, ErrorEntry>, _, _>(track_file, |entries| {
        entries.remove(cmd_key)
    })
    .ok()
    .flatten();

    if let Some(prev_error) = prev_error {
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200))),
            OutcomeEvent::new(
                OutcomeKind::ErrorResolved,
                format!("Recovered command: {}", truncate(command, 200)),
            )
            .with_command(truncate(command, 500))
            .with_signal_type("error_resolved"),
        );
        record_outcome(hash, outcome);

        if command_exists("hyphae") {
            let content = format!(
                "Fixed: {}\nPrevious error: {}",
                &command[..command.len().min(200)],
                &prev_error.error[..prev_error.error.len().min(300)]
            );
            let project = project_name_for_cwd(scope_cwd);
            store_in_hyphae(
                "errors/resolved",
                &content,
                Importance::High,
                project.as_deref(),
            );
            log_scoped_hyphae_feedback_signal(
                scope_cwd,
                "error_resolved",
                1,
                "cortina.post_tool_use.error_resolution",
                Some(&truncate(command, 200)),
            );
        }
    }
}

fn store_error_in_hyphae(command: &str, output: &str, scope_cwd: Option<&str>) {
    let content = format!(
        "Command: {}\nError: {}",
        &command[..command.len().min(200)],
        &output[..output.len().min(500)]
    );
    store_in_hyphae(
        "errors/active",
        &content,
        Importance::Medium,
        project_name_for_cwd(scope_cwd).as_deref(),
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Edit tracking and corrections
// ─────────────────────────────────────────────────────────────────────────

fn load_and_clean_edits(track_file: &Path) -> Vec<EditEntry> {
    let mut edits: Vec<EditEntry> = load_json_file(track_file).unwrap_or_default();
    let cutoff = current_timestamp_ms().saturating_sub(CLEANUP_AGE_MS);

    edits.retain(|e| e.timestamp > cutoff);
    edits
}

fn track_edit(track_file: &Path, file_path: &str, old_str: &str, new_str: &str) {
    let now = current_timestamp_ms();
    let cutoff = now.saturating_sub(CLEANUP_AGE_MS);

    let _ = update_json_file::<Vec<EditEntry>, _, _>(track_file, |edits| {
        edits.retain(|e| e.timestamp > cutoff);
        edits.push(EditEntry {
            file: file_path.to_string(),
            old_string: old_str.chars().take(200).collect(),
            new_string: new_str.chars().take(200).collect(),
            timestamp: now,
        });
    });
}

fn find_correction(file_path: &str, old_str: &str, track_file: &Path) -> Option<EditEntry> {
    let edits = load_and_clean_edits(track_file);
    let cutoff = current_timestamp_ms().saturating_sub(CORRECTION_WINDOW_MS);

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
    scope_cwd: Option<&str>,
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

    let project = project_name_for_cwd(scope_cwd);
    store_in_hyphae(
        "corrections",
        &content,
        Importance::High,
        project.as_deref(),
    );
    log_scoped_hyphae_feedback_signal(
        scope_cwd,
        "correction",
        -1,
        "cortina.post_tool_use.correction",
        Some(&truncate(file_path, 200)),
    );
}

fn log_validation_success(
    command: &str,
    exit_code: Option<i32>,
    hash: &str,
    scope_cwd: Option<&str>,
) {
    let Some((signal_type, signal_value, source)) =
        successful_validation_feedback(command, exit_code)
    else {
        return;
    };

    let outcome = annotate_outcome_with_session(
        ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200))),
        OutcomeEvent::new(
            OutcomeKind::ValidationPassed,
            format!("Validation passed: {}", truncate(command, 200)),
        )
        .with_command(truncate(command, 500))
        .with_signal_type(signal_type),
    );
    record_outcome(hash, outcome);

    log_scoped_hyphae_feedback_signal(
        scope_cwd,
        signal_type,
        signal_value,
        source,
        Some(&truncate(command, 200)),
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Rhizome export and Hyphae ingest tracking
// ─────────────────────────────────────────────────────────────────────────

fn get_pending_files_path(hash: &str) -> std::path::PathBuf {
    temp_state_path("pending-exports", hash, "json")
}

fn get_pending_documents_path(hash: &str) -> std::path::PathBuf {
    temp_state_path("pending-ingest", hash, "json")
}

#[cfg(test)]
fn get_pending_files(hash: &str) -> Vec<String> {
    load_json_file(get_pending_files_path(hash)).unwrap_or_default()
}

#[cfg(test)]
fn get_pending_documents(hash: &str) -> Vec<String> {
    load_json_file(get_pending_documents_path(hash)).unwrap_or_default()
}

fn track_pending_file(file_path: &str, hash: &str) {
    let path = get_pending_files_path(hash);
    let _ = update_json_file::<Vec<String>, _, _>(&path, |files| {
        if !files.iter().any(|existing| existing == file_path) {
            files.push(file_path.to_string());
        }
    });
}

fn track_pending_document(file_path: &str, hash: &str) {
    let path = get_pending_documents_path(hash);
    let _ = update_json_file::<Vec<String>, _, _>(&path, |files| {
        if !files.iter().any(|existing| existing == file_path) {
            files.push(file_path.to_string());
        }
    });
}

#[cfg(test)]
fn clear_pending_files(hash: &str) {
    let path = get_pending_files_path(hash);
    let _ = crate::utils::remove_file_with_lock(&path);
}

#[cfg(test)]
fn clear_pending_documents(hash: &str) {
    let path = get_pending_documents_path(hash);
    let _ = crate::utils::remove_file_with_lock(&path);
}

fn check_and_trigger_exports(hash: &str, scope_cwd: Option<&str>) {
    if !command_exists("rhizome") {
        return;
    }

    let pending_files = take_pending_files_batch(hash);
    if !pending_files.is_empty() {
        if !spawn_async_checked("rhizome", &["export"]) {
            requeue_pending_files(&pending_files, hash);
            return;
        }
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some("rhizome export")),
            OutcomeEvent::new(
                OutcomeKind::KnowledgeExported,
                format!("Triggered rhizome export for {} files", pending_files.len()),
            )
            .with_command("rhizome export"),
        );
        record_outcome(hash, outcome);
    }
}

fn trigger_hyphae_ingest(documents: &[String], hash: &str, scope_cwd: Option<&str>) -> Vec<String> {
    let mut spawned = 0usize;
    let mut failed_docs = Vec::new();
    for doc in documents {
        if spawn_async_checked("hyphae", &["ingest-file", doc]) {
            spawned += 1;
        } else {
            failed_docs.push(doc.clone());
        }
    }
    if spawned == 0 {
        return failed_docs;
    }
    let outcome = annotate_outcome_with_session(
        ensure_scoped_hyphae_session(scope_cwd, Some("hyphae ingest-file")),
        OutcomeEvent::new(
            OutcomeKind::DocumentIngested,
            format!("Triggered hyphae ingest for {spawned} documents"),
        )
        .with_command("hyphae ingest-file"),
    );
    record_outcome(hash, outcome);
    failed_docs
}

fn take_pending_files_batch(hash: &str) -> Vec<String> {
    take_pending_batch(&get_pending_files_path(hash), EXPORT_THRESHOLD)
}

fn take_pending_documents_batch(hash: &str) -> Vec<String> {
    take_pending_batch(&get_pending_documents_path(hash), INGEST_THRESHOLD)
}

fn take_pending_batch(path: &Path, threshold: usize) -> Vec<String> {
    update_json_file::<Vec<String>, _, _>(path, |entries| {
        if entries.len() < threshold {
            Vec::new()
        } else {
            std::mem::take(entries)
        }
    })
    .unwrap_or_default()
}

fn requeue_pending_files(files: &[String], hash: &str) {
    requeue_pending_batch(&get_pending_files_path(hash), files);
}

fn requeue_pending_documents(files: &[String], hash: &str) {
    requeue_pending_batch(&get_pending_documents_path(hash), files);
}

fn requeue_pending_batch(path: &Path, files: &[String]) {
    let _ = update_json_file::<Vec<String>, _, _>(path, |entries| {
        for file in files {
            if !entries.iter().any(|existing| existing == file) {
                entries.push(file.clone());
            }
        }
    });
}

fn truncate(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn annotate_outcome_with_session(
    session: Option<SessionState>,
    outcome: OutcomeEvent,
) -> OutcomeEvent {
    match session {
        Some(state) => outcome.with_session(state.session_id, state.project),
        None => outcome,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

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
        let malformed = r"{ invalid json }";
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

    #[test]
    fn test_log_validation_success_records_structured_outcome() {
        let hash = format!("test-validation-{}", std::process::id());
        crate::outcomes::clear_outcomes(&hash);

        log_validation_success("cargo test", Some(0), &hash, None);

        let outcomes = crate::outcomes::load_outcomes(&hash);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].kind, OutcomeKind::ValidationPassed);
        assert_eq!(outcomes[0].signal_type.as_deref(), Some("test_passed"));

        crate::outcomes::clear_outcomes(&hash);
    }

    #[test]
    fn test_handle_file_edits_tracks_new_file_when_old_string_is_empty() {
        let cwd = format!("/tmp/cortina-new-file-{}", std::process::id());
        let hash = scope_hash(Some(&cwd));
        clear_pending_files(&hash);
        clear_pending_documents(&hash);

        handle_file_edits(&FileEditEvent {
            file_path: "docs/new.md".to_string(),
            old_string: String::new(),
            new_string: "# hello".to_string(),
            cwd: Some(cwd.clone()),
        });

        let pending_files = get_pending_files(&hash);
        let pending_docs = get_pending_documents(&hash);
        assert!(pending_files.iter().any(|path| path == "docs/new.md"));
        assert!(pending_docs.iter().any(|path| path == "docs/new.md"));

        clear_pending_files(&hash);
        clear_pending_documents(&hash);
    }

    #[test]
    fn test_track_pending_file_deduplicates_paths() {
        let hash = format!("test-pending-dedupe-{}", std::process::id());
        clear_pending_files(&hash);

        track_pending_file("src/lib.rs", &hash);
        track_pending_file("src/lib.rs", &hash);

        let pending_files = get_pending_files(&hash);
        assert_eq!(pending_files, vec!["src/lib.rs".to_string()]);

        clear_pending_files(&hash);
    }

    #[test]
    fn test_track_pending_document_deduplicates_paths() {
        let hash = format!("test-pending-doc-dedupe-{}", std::process::id());
        clear_pending_documents(&hash);

        track_pending_document("docs/guide.md", &hash);
        track_pending_document("docs/guide.md", &hash);

        let pending_docs = get_pending_documents(&hash);
        assert_eq!(pending_docs, vec!["docs/guide.md".to_string()]);

        clear_pending_documents(&hash);
    }

    #[test]
    fn test_track_pending_file_preserves_concurrent_writes() {
        let hash = format!("test-pending-concurrent-{}", std::process::id());
        clear_pending_files(&hash);

        let workers = 10;
        let barrier = Arc::new(Barrier::new(workers));
        let mut handles = Vec::new();

        for idx in 0..workers {
            let hash = hash.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                track_pending_file(&format!("src/file-{idx}.rs"), &hash);
            }));
        }

        for handle in handles {
            handle.join().expect("thread should finish cleanly");
        }

        let pending_files = get_pending_files(&hash);
        assert_eq!(pending_files.len(), workers);
        for idx in 0..workers {
            assert!(pending_files.iter().any(|path| path == &format!("src/file-{idx}.rs")));
        }

        clear_pending_files(&hash);
    }

    #[test]
    fn test_take_pending_documents_batch_drains_only_when_threshold_met() {
        let hash = format!("test-pending-doc-batch-{}", std::process::id());
        clear_pending_documents(&hash);

        track_pending_document("docs/a.md", &hash);
        track_pending_document("docs/b.md", &hash);

        assert!(take_pending_documents_batch(&hash).is_empty());
        assert_eq!(get_pending_documents(&hash).len(), 2);

        track_pending_document("docs/c.md", &hash);

        let batch = take_pending_documents_batch(&hash);
        assert_eq!(batch.len(), 3);
        assert!(get_pending_documents(&hash).is_empty());

        clear_pending_documents(&hash);
    }
}

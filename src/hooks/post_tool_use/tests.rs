use std::sync::{Arc, Barrier};
use std::thread;

use crate::events::{FileEditEvent, OutcomeKind};

use super::bash::log_validation_success;
use super::edits::handle_file_edits;
use super::pending::{
    clear_pending_documents, clear_pending_files, get_pending_documents, get_pending_files,
    hyphae_ingest_args, take_pending_documents_batch, track_pending_document, track_pending_file,
};
use super::*;

#[test]
fn parse_bash_hook_event_valid() {
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
fn parse_bash_hook_with_error_exit_code() {
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

#[test]
fn parse_edit_hook_event_valid() {
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

#[test]
fn handle_malformed_json_returns_ok() {
    assert!(handle(r"{ invalid json }").is_ok());
}

#[test]
fn handle_empty_json_returns_ok() {
    assert!(handle("").is_ok());
}

#[test]
fn handle_valid_bash_event_dispatches_to_bash_tracking() {
    let cwd = format!("/tmp/cortina-post-tool-use-{}", std::process::id());
    let hash = crate::utils::scope_hash(Some(&cwd));
    crate::outcomes::clear_outcomes(&hash);

    let json = format!(
        r#"{{
            "tool_name": "Bash",
            "cwd": "{cwd}",
            "tool_input": {{"command": "cargo test"}},
            "tool_output": {{"output": "ok", "exit_code": 0}}
        }}"#
    );

    assert!(handle(&json).is_ok());

    let outcomes = crate::outcomes::load_outcomes(&hash);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].kind, OutcomeKind::ValidationPassed);

    crate::outcomes::clear_outcomes(&hash);
}

#[test]
fn handle_valid_json_with_unknown_tool() {
    let json_str = r#"{
        "tool_name": "UnknownTool",
        "tool_input": {}
    }"#;

    assert!(handle(json_str).is_ok());
}

#[test]
fn log_validation_success_records_structured_outcome() {
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
fn handle_file_edits_tracks_new_file_when_old_string_is_empty() {
    let cwd = format!("/tmp/cortina-new-file-{}", std::process::id());
    let hash = crate::utils::scope_hash(Some(&cwd));
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
fn track_pending_file_deduplicates_paths() {
    let hash = format!("test-pending-dedupe-{}", std::process::id());
    clear_pending_files(&hash);

    track_pending_file("src/lib.rs", &hash);
    track_pending_file("src/lib.rs", &hash);

    let pending_files = get_pending_files(&hash);
    assert_eq!(pending_files, vec!["src/lib.rs".to_string()]);

    clear_pending_files(&hash);
}

#[test]
fn track_pending_document_deduplicates_paths() {
    let hash = format!("test-pending-doc-dedupe-{}", std::process::id());
    clear_pending_documents(&hash);

    track_pending_document("docs/guide.md", &hash);
    track_pending_document("docs/guide.md", &hash);

    let pending_docs = get_pending_documents(&hash);
    assert_eq!(pending_docs, vec!["docs/guide.md".to_string()]);

    clear_pending_documents(&hash);
}

#[test]
fn track_pending_file_preserves_concurrent_writes() {
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
        assert!(
            pending_files
                .iter()
                .any(|path| path == &format!("src/file-{idx}.rs"))
        );
    }

    clear_pending_files(&hash);
}

#[test]
fn take_pending_documents_batch_drains_only_when_threshold_met() {
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

#[test]
fn hyphae_pending_ingest_uses_current_cli_subcommand() {
    assert_eq!(
        hyphae_ingest_args("docs/guide.md"),
        ["ingest", "docs/guide.md"]
    );
}

#[test]
fn annotate_outcome_with_session_carries_exact_identity_when_available() {
    let session = crate::utils::SessionState {
        session_id: "ses_demo".to_string(),
        project: "demo".to_string(),
        project_root: Some("/tmp/demo".to_string()),
        worktree_id: Some("git:demo".to_string()),
        legacy_scope: None,
        started_at: 1,
    };

    let outcome = annotate_outcome_with_session(
        Some(session),
        crate::events::OutcomeEvent::new(OutcomeKind::ValidationPassed, "cargo test passed"),
    );

    assert_eq!(outcome.session_id.as_deref(), Some("ses_demo"));
    assert_eq!(outcome.project.as_deref(), Some("demo"));
    assert_eq!(outcome.project_root.as_deref(), Some("/tmp/demo"));
    assert_eq!(outcome.worktree_id.as_deref(), Some("git:demo"));
}

use crate::events::{OutcomeEvent, OutcomeKind};

use super::handle;
use super::summary::{
    TranscriptSummary, filter_outcomes_for_session, format_structured_outcome_attribution,
    has_unresolved_errors, merge_structured_outcomes,
};
use super::transcript::parse_jsonl_transcript;

#[test]
fn parse_jsonl_transcript_valid() {
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
fn parse_jsonl_transcript_counts_tools() {
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
fn parse_jsonl_transcript_counts_errors() {
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
fn parse_jsonl_transcript_ignores_error_handling_prose() {
    let jsonl = r#"{"type": "human", "text": "Task"}
{"type": "tool_result", "content": "Improved error handling and validation flow"}
{"type": "assistant", "text": "Done"}
"#;

    let summary = parse_jsonl_transcript(jsonl);

    assert_eq!(summary.errors_encountered, 0);
}

#[test]
fn parse_jsonl_transcript_empty_input() {
    let summary = parse_jsonl_transcript("");

    assert_eq!(summary.task_desc, "Session work");
    assert_eq!(summary.outcome, "Work completed");
    assert_eq!(summary.errors_encountered, 0);
    assert!(summary.files_modified.is_empty());
}

#[test]
fn parse_jsonl_transcript_extracts_files() {
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
fn merge_structured_outcomes_enriches_summary() {
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
fn format_structured_outcome_attribution_counts_by_kind() {
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
fn filter_outcomes_for_session_prefers_matching_session_id() {
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
fn filter_outcomes_for_session_keeps_current_unattributed_outcomes_only() {
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
fn has_unresolved_errors_stays_true_after_non_error_outcome() {
    let outcomes = vec![
        OutcomeEvent::new(OutcomeKind::ErrorDetected, "command failed"),
        OutcomeEvent::new(OutcomeKind::DocumentIngested, "docs ingested"),
    ];

    assert!(has_unresolved_errors(&outcomes));
}

#[test]
fn handle_accepts_valid_stop_envelope() {
    let json = format!(
        r#"{{
            "cwd": "/tmp/cortina-stop-{}",
            "transcript_path": null
        }}"#,
        std::process::id()
    );

    assert!(handle(&json).is_ok());
}

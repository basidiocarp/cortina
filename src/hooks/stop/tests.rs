use crate::events::{CausalSignal, OutcomeEvent, OutcomeKind};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use super::handle;
use super::summary::{
    TranscriptSummary, filter_outcomes_for_session, format_structured_outcome_attribution,
    has_unresolved_errors, merge_structured_outcomes,
};
use super::transcript::parse_jsonl_transcript;
use super::{check_handoff_completion, check_handoff_staleness};

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
        project_root: Some("/tmp/demo".to_string()),
        worktree_id: Some("git:demo".to_string()),
        legacy_scope: None,
        started_at: 100,
        memory_protocol: None,
    };
    let mut project_only = OutcomeEvent::new(OutcomeKind::ValidationPassed, "project-only");
    project_only.project = Some("demo".to_string());
    project_only.timestamp = 120;
    let mut unattributed = OutcomeEvent::new(OutcomeKind::ValidationPassed, "unattributed");
    unattributed.timestamp = 130;
    let outcomes = vec![
        OutcomeEvent::new(OutcomeKind::ErrorDetected, "old").with_session("ses_old", "demo"),
        OutcomeEvent::new(OutcomeKind::ValidationPassed, "current")
            .with_session("ses_current", "demo"),
        project_only,
        unattributed,
    ];

    let filtered = filter_outcomes_for_session(&outcomes, Some(&session), "demo");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].summary, "current");
}

#[test]
fn filter_outcomes_for_session_ignores_unattributed_outcomes_for_structured_sessions() {
    let session = crate::utils::SessionState {
        session_id: "ses_current".to_string(),
        project: "demo".to_string(),
        project_root: Some("/tmp/demo".to_string()),
        worktree_id: Some("git:demo".to_string()),
        legacy_scope: None,
        started_at: 100_000,
        memory_protocol: None,
    };
    let mut old = OutcomeEvent::new(OutcomeKind::ErrorDetected, "old unattributed");
    old.timestamp = 1_000;
    let mut current = OutcomeEvent::new(OutcomeKind::ValidationPassed, "current unattributed");
    current.timestamp = 100_100;
    let outcomes = vec![old, current];

    let filtered = filter_outcomes_for_session(&outcomes, Some(&session), "demo");

    assert!(filtered.is_empty());
}

#[test]
fn filter_outcomes_for_session_accepts_exact_identity_matches_without_session_id() {
    let session = crate::utils::SessionState {
        session_id: "ses_current".to_string(),
        project: "demo".to_string(),
        project_root: Some("/tmp/demo".to_string()),
        worktree_id: Some("git:demo".to_string()),
        legacy_scope: None,
        started_at: 1_000,
        memory_protocol: None,
    };
    let mut identity_scoped = OutcomeEvent::new(OutcomeKind::ValidationPassed, "identity match");
    identity_scoped.project_root = Some("/tmp/demo".to_string());
    identity_scoped.worktree_id = Some("git:demo".to_string());
    identity_scoped.timestamp = 1_010;

    let filtered = filter_outcomes_for_session(&[identity_scoped], Some(&session), "demo");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].summary, "identity match");
}

#[test]
fn filter_outcomes_for_session_ignores_clock_skewed_unattributed_outcomes_for_structured_sessions()
{
    let session = crate::utils::SessionState {
        session_id: "ses_current".to_string(),
        project: "demo".to_string(),
        project_root: Some("/tmp/demo".to_string()),
        worktree_id: Some("git:demo".to_string()),
        legacy_scope: None,
        started_at: 1_000,
        memory_protocol: None,
    };
    let mut near_start = OutcomeEvent::new(OutcomeKind::ValidationPassed, "near start");
    near_start.timestamp = 980;

    let filtered = filter_outcomes_for_session(&[near_start], Some(&session), "demo");

    assert!(filtered.is_empty());
}

#[test]
fn filter_outcomes_for_session_preserves_causal_attribution() {
    let session = crate::utils::SessionState {
        session_id: "ses_current".to_string(),
        project: "demo".to_string(),
        project_root: Some("/tmp/demo".to_string()),
        worktree_id: Some("git:demo".to_string()),
        legacy_scope: None,
        started_at: 100,
        memory_protocol: None,
    };
    let caused_by = CausalSignal::new("error_detected", "Command failed: cargo test", 90)
        .with_command("cargo test");
    let mut resolved =
        OutcomeEvent::new(OutcomeKind::ErrorResolved, "Recovered command: cargo test");
    resolved.session_id = Some("ses_current".to_string());
    resolved.caused_by = Some(caused_by.clone());

    let filtered = filter_outcomes_for_session(&[resolved], Some(&session), "demo");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].caused_by, Some(caused_by));
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
fn has_unresolved_errors_uses_latest_error_state() {
    let outcomes = vec![
        OutcomeEvent::new(OutcomeKind::ErrorDetected, "first failure"),
        OutcomeEvent::new(OutcomeKind::ErrorResolved, "fixed first failure"),
        OutcomeEvent::new(OutcomeKind::ErrorDetected, "second failure"),
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

#[test]
fn check_handoff_staleness_warns_on_overlap_with_unchecked_items() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".handoffs/cortina")).unwrap();
    fs::write(
        dir.path().join(".handoffs/HANDOFFS.md"),
        r"# Handoffs Index

| Handoff | Status | Priority | Depends On |
|---------|--------|----------|------------|
| [Stale Handoff Detection](cortina/stale-handoff-detection.md) | Ready | High | — |
",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join(".handoffs/cortina/stale-handoff-detection.md"),
        r"# Handoff

#### Files to modify

**`cortina/src/hooks/stop.rs`** — add staleness detection

#### Checklist

- [ ] Add `check_handoff_staleness`
",
    )
    .unwrap();

    let warnings = check_handoff_staleness(
        &[String::from("cortina/src/hooks/stop.rs")],
        &PathBuf::from(dir.path()),
    );

    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .overlapping_files
            .iter()
            .any(|path| path == "cortina/src/hooks/stop.rs")
    );
    assert!(
        warnings[0]
            .unchecked_items
            .iter()
            .any(|item| item.contains("check_handoff_staleness"))
    );
}

#[test]
fn check_handoff_staleness_ignores_overlap_from_checked_items() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".handoffs/cortina")).unwrap();
    fs::write(
        dir.path().join(".handoffs/HANDOFFS.md"),
        r"# Handoffs Index

| Handoff | Status | Priority | Depends On |
|---------|--------|----------|------------|
| [Mixed Handoff](cortina/mixed.md) | Ready | High | — |
",
    )
    .unwrap();
    fs::write(
        dir.path().join(".handoffs/cortina/mixed.md"),
        r"# Handoff

### Step 1

#### Files to modify

**`cortina/src/hooks/stop.rs`**

- [x] Finish `check_handoff_staleness`

### Step 2

#### Files to modify

**`cortina/src/main.rs`**

- [ ] Tighten dispatch wording
",
    )
    .unwrap();

    let warnings = check_handoff_staleness(
        &[String::from("cortina/src/hooks/stop.rs")],
        &PathBuf::from(dir.path()),
    );

    assert!(warnings.is_empty());
}

#[test]
fn check_handoff_completion_warns_for_modified_handoffs() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".handoffs/cross-project")).unwrap();
    fs::write(
        dir.path().join(".handoffs/HANDOFFS.md"),
        "# Handoffs Index\n",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join(".handoffs/cross-project/handoff-checkbox-enforcement.md"),
        r"# Handoff

- [ ] Tighten stop hook

**Output:**
<!-- PASTE START -->

<!-- PASTE END -->
",
    )
    .unwrap();

    let warnings = check_handoff_completion(
        &[PathBuf::from(
            ".handoffs/cross-project/handoff-checkbox-enforcement.md",
        )],
        dir.path(),
    );

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("unchecked checklist items"));
    assert!(warnings[0].contains("empty paste markers"));
    assert!(warnings[0].contains("claiming completion"));
}

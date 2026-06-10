use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::events::{NormalizedLifecycleEvent, OutcomeEvent, OutcomeKind};
use crate::outcomes::{load_outcomes, record_outcome};
use crate::signals::PreCompactSnapshot;
use crate::utils::{
    Importance, command_exists, current_agent_id_for_cwd, current_task_id_for_cwd,
    current_timestamp_ms, ensure_scoped_hyphae_session, load_json_file, project_name_for_cwd,
    scope_hash, store_compact_summary_artifact, store_in_hyphae, temp_state_path, update_json_file,
};

use super::parse_error::parse_or_allow;
use super::post_tool_use::{get_pending_documents, get_pending_files};

const MAX_RECORDED_SNAPSHOTS: usize = 16;
const SNAPSHOT_SESSION_TASK: &str = "pre compact snapshot";
const SUMMARY_REQUEST: &str = "Please summarize the current work before compaction.";

/// Snapshot of an active error during pre-compaction.
/// Reads from the same file written by [`ErrorEntry`] in `hooks/post_tool_use/bash.rs`.
/// Field names must stay in sync; only a subset is needed here.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
struct ActiveErrorEntry {
    command: String,
    error: String,
    timestamp: u64,
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> anyhow::Result<()> {
    let Some(envelope) = parse_or_allow(input) else {
        return Ok(());
    };

    let Some(event) = envelope.pre_compact_event() else {
        return Ok(());
    };

    capture_pre_compact(&event);
    Ok(())
}

fn capture_pre_compact(event: &crate::events::PreCompactEvent) {
    let hash = scope_hash(Some(&event.cwd));
    let active_errors = load_active_errors(&hash);
    let files_modified = collect_modified_files(&hash);
    let active_task_id = current_task_id_for_cwd(Some(&event.cwd));
    let signal_summary = build_signal_summary(&hash);
    let resume_hint = build_resume_hint(
        &active_errors,
        &files_modified,
        active_task_id.as_deref(),
        &signal_summary,
    );
    let content = compaction_snapshot_content(
        event,
        &active_errors,
        &files_modified,
        active_task_id.as_deref(),
        &signal_summary,
    );

    if !remember_snapshot_capture(&hash, &content) {
        return;
    }

    // Construct PreCompactSnapshot for Layer 1 (logged at debug level).
    let open_errors: Vec<String> = active_errors
        .values()
        .map(|e| format!("{}: {}", e.command, e.error))
        .collect();
    let mut signal_counts = std::collections::HashMap::new();
    for (k, v) in &signal_summary {
        signal_counts.insert(k.clone(), *v);
    }
    let pre_compact_snapshot = PreCompactSnapshot {
        active_files: files_modified.clone(),
        open_errors,
        resume_hint: resume_hint.clone(),
        active_task_id: active_task_id.clone(),
        signal_counts,
    };
    tracing::debug!("pre_compact_snapshot: {:?}", pre_compact_snapshot);

    if command_exists("hyphae") {
        // Snapshot the prior outcomes before the snapshot-marker outcome below is
        // recorded, so memory extraction lifts real session outcomes and not the
        // bookkeeping "snapshot stored" marker this function is about to write.
        let prior_outcomes = load_outcomes(&hash);

        let _ = ensure_scoped_hyphae_session(Some(&event.cwd), Some(SNAPSHOT_SESSION_TASK));
        let project = project_name_for_cwd(Some(&event.cwd));
        let topic = project.as_deref().map_or_else(
            || "session/compaction-snapshot".to_string(),
            |name| format!("context/{name}/pre-compact"),
        );
        let agent_id = current_agent_id_for_cwd(Some(&event.cwd));
        store_in_hyphae(
            &topic,
            &content,
            Importance::High,
            project.as_deref(),
            agent_id.as_deref(),
        );
        let outcome = OutcomeEvent::new(
            OutcomeKind::KnowledgeExported,
            format!("pre-compact snapshot stored in hyphae ({topic})"),
        )
        .with_signal_type("compaction_marker");
        let _ = record_outcome(&hash, outcome);

        // Emit a typed compact_summary artifact alongside the snapshot. Failures
        // are isolated — the artifact path logs and continues independently of
        // the snapshot store above.
        let artifact = compact_summary_artifact_payload(
            event,
            &files_modified,
            active_task_id.as_deref(),
            &signal_summary,
        );
        store_compact_summary_artifact(&artifact, project.as_deref());

        // Lift discrete high-signal outcomes into durable memory. Runs last so the
        // existing snapshot and artifact stores complete first; fail-open.
        extract_and_store_memories(event, &prior_outcomes);
    }
}

/// Build the `(topic, content)` pairs for the high-signal outcomes worth lifting
/// into durable memory at a compaction boundary. Pure (no I/O) so the selection,
/// most-recent ordering, bound, and topic-formatting logic is unit-testable
/// without a live hyphae backend.
///
/// Only `ErrorResolved`, `KnowledgeExported`, and `ValidationPassed` are kept;
/// the most recent are preferred and the result is capped at a small constant so
/// a long session does not fan out into excessive hyphae writes.
fn pre_compact_memory_entries(
    outcomes: &[OutcomeEvent],
    project: Option<&str>,
) -> Vec<(String, String)> {
    const MAX_EXTRACTED_MEMORIES: usize = 8;
    outcomes
        .iter()
        .rev()
        .filter_map(|outcome| {
            // Skip the self-written compaction marker to avoid accumulating duplicates.
            if outcome.signal_type.as_deref() == Some("compaction_marker") {
                return None;
            }
            let kind_label = match outcome.kind {
                OutcomeKind::ErrorResolved => "error-resolved",
                OutcomeKind::KnowledgeExported => "knowledge-exported",
                OutcomeKind::ValidationPassed => "validation-passed",
                _ => return None,
            };
            let content = match &outcome.command {
                Some(command) => format!("{}\ncommand: {command}", outcome.summary),
                None => outcome.summary.clone(),
            };
            let topic = project.map_or_else(
                || format!("session/pre-compact/memory/{kind_label}"),
                |name| format!("context/{name}/pre-compact/memory/{kind_label}"),
            );
            Some((topic, content))
        })
        .take(MAX_EXTRACTED_MEMORIES)
        .collect()
}

/// Lift discrete high-signal session outcomes into durable hyphae memories at the
/// compaction boundary, so the most useful facts survive context compression as
/// individually-retrievable entries rather than only as one snapshot blob.
///
/// Runs only inside the `command_exists("hyphae")` block and only after the
/// snapshot-dedup guard in `capture_pre_compact` has fired, so a duplicate
/// compaction never re-extracts. Fail-open: `store_in_hyphae` is fire-and-forget,
/// so any individual store problem is swallowed and can never break compaction.
fn extract_and_store_memories(event: &crate::events::PreCompactEvent, outcomes: &[OutcomeEvent]) {
    let project = project_name_for_cwd(Some(&event.cwd));
    let agent_id = current_agent_id_for_cwd(Some(&event.cwd));
    for (topic, content) in pre_compact_memory_entries(outcomes, project.as_deref()) {
        store_in_hyphae(
            &topic,
            &content,
            Importance::High,
            project.as_deref(),
            agent_id.as_deref(),
        );
    }
}

fn build_resume_hint(
    active_errors: &BTreeMap<String, ActiveErrorEntry>,
    files_modified: &[String],
    active_task_id: Option<&str>,
    signal_summary: &BTreeMap<String, usize>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(task_id) = active_task_id {
        parts.push(format!("Working on task {task_id}."));
    }

    if !files_modified.is_empty() {
        let count = files_modified.len();
        let sample = files_modified
            .iter()
            .take(3)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        if count > 3 {
            parts.push(format!("Modified {count} files including {sample}."));
        } else {
            parts.push(format!("Modified {sample}."));
        }
    }

    if !active_errors.is_empty() {
        let error_count = active_errors.len();
        let sample_cmd = active_errors
            .keys()
            .next()
            .map_or("unknown", String::as_str);
        parts.push(format!(
            "{error_count} unresolved error(s) including from `{sample_cmd}`."
        ));
    } else if signal_summary
        .get("validation_passed")
        .copied()
        .unwrap_or(0)
        > 0
    {
        parts.push("Recent validations passed.".to_string());
    }

    if parts.is_empty() {
        "Session in progress.".to_string()
    } else {
        parts.join(" ")
    }
}

fn compaction_snapshot_content(
    event: &crate::events::PreCompactEvent,
    active_errors: &BTreeMap<String, ActiveErrorEntry>,
    files_modified: &[String],
    active_task_id: Option<&str>,
    signal_summary: &BTreeMap<String, usize>,
) -> String {
    let resume_hint = build_resume_hint(
        active_errors,
        files_modified,
        active_task_id,
        signal_summary,
    );
    let active_errors: Vec<_> = active_errors
        .values()
        .map(|error| json!({ "command": error.command, "error": error.error }))
        .collect();
    let mut files_modified = files_modified.to_vec();
    files_modified.sort();

    json!({
        "type": "compaction_snapshot",
        "normalized_lifecycle_event": NormalizedLifecycleEvent::from_pre_compact(event),
        "session_id": event.session_id,
        "cwd": event.cwd,
        "trigger": event.trigger,
        "custom_instructions": event.custom_instructions,
        "summary_request": SUMMARY_REQUEST,
        "active_errors": active_errors,
        "files_modified": files_modified,
        "transcript_path": event.transcript_path,
        "active_task_id": active_task_id,
        "signal_summary": signal_summary,
        "resume_hint": resume_hint,
    })
    .to_string()
}

/// Build a typed `compact_summary` artifact payload for Hyphae.
///
/// The topic used by [`store_compact_summary_artifact`] is
/// `artifact/compact_summary/{session_id}`, making it queryable by convention.
fn compact_summary_artifact_payload(
    event: &crate::events::PreCompactEvent,
    files_modified: &[String],
    active_task_id: Option<&str>,
    signal_summary: &BTreeMap<String, usize>,
) -> String {
    let active_errors = load_active_errors(&crate::utils::scope_hash(Some(&event.cwd)));
    let resume_hint = build_resume_hint(
        &active_errors,
        files_modified,
        active_task_id,
        signal_summary,
    );
    let mut files_modified = files_modified.to_vec();
    files_modified.sort();
    json!({
        "artifact_type": "compact_summary",
        "session_id": event.session_id,
        "project": project_name_for_cwd(Some(&event.cwd)),
        "files_modified": files_modified,
        "active_task_id": active_task_id,
        "signal_summary": signal_summary,
        "summary_request": SUMMARY_REQUEST,
        "resume_hint": resume_hint,
        "created_at": current_timestamp_ms(),
    })
    .to_string()
}

/// Count outcomes by kind label for use in compaction snapshots.
fn build_signal_summary(hash: &str) -> BTreeMap<String, usize> {
    let outcomes = load_outcomes(hash);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for outcome in &outcomes {
        *counts.entry(outcome.kind.label().to_string()).or_insert(0) += 1;
    }
    counts
}

fn collect_modified_files(hash: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    files.extend(get_pending_files(hash));
    files.extend(get_pending_documents(hash));
    files.into_iter().collect()
}

fn load_active_errors(hash: &str) -> BTreeMap<String, ActiveErrorEntry> {
    let path = temp_state_path("errors", hash, "json");
    load_json_file::<BTreeMap<String, ActiveErrorEntry>>(path).unwrap_or_default()
}

fn snapshot_capture_state_path(hash: &str) -> PathBuf {
    temp_state_path("compaction-snapshots", hash, "json")
}

fn remember_snapshot_capture(hash: &str, content: &str) -> bool {
    update_json_file::<Vec<String>, _, _>(&snapshot_capture_state_path(hash), |snapshots| {
        if snapshots.iter().any(|existing| existing == content) {
            return false;
        }

        snapshots.push(content.to_string());
        if snapshots.len() > MAX_RECORDED_SNAPSHOTS {
            let overflow = snapshots.len().saturating_sub(MAX_RECORDED_SNAPSHOTS);
            snapshots.drain(0..overflow);
        }
        true
    })
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::PreCompactEvent;
    use crate::utils::remove_file_with_lock;

    #[test]
    fn builds_compaction_snapshot_content() {
        let event = PreCompactEvent {
            session_id: "abc123".to_string(),
            cwd: "/tmp/demo".to_string(),
            trigger: "manual".to_string(),
            custom_instructions: Some("summarize current work".to_string()),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
        };
        let mut active_errors = BTreeMap::new();
        active_errors.insert(
            "cargo test".to_string(),
            ActiveErrorEntry {
                command: "cargo test".to_string(),
                error: "FAILED".to_string(),
                timestamp: 123,
            },
        );

        let content = compaction_snapshot_content(
            &event,
            &active_errors,
            &["src/main.rs".to_string(), "README.md".to_string()],
            Some("task-42"),
            &BTreeMap::from([("error_detected".to_string(), 2usize)]),
        );
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid json");

        assert!(content.contains(r#""type":"compaction_snapshot""#));
        assert_eq!(
            parsed["normalized_lifecycle_event"]["category"].as_str(),
            Some("compaction")
        );
        assert_eq!(
            parsed["normalized_lifecycle_event"]["event_name"].as_str(),
            Some("pre_compact")
        );
        assert!(content.contains(
            r#""summary_request":"Please summarize the current work before compaction.""#
        ));
        assert!(content.contains(r#""trigger":"manual""#));
        assert!(content.contains(r#""files_modified":["README.md","src/main.rs"]"#));
        assert_eq!(parsed["active_task_id"].as_str(), Some("task-42"));
        assert_eq!(parsed["signal_summary"]["error_detected"].as_u64(), Some(2));
        assert!(parsed["resume_hint"].is_string());
        let hint = parsed["resume_hint"]
            .as_str()
            .expect("resume_hint is string");
        assert!(!hint.is_empty(), "resume_hint should not be empty");
    }

    #[test]
    fn compact_summary_artifact_payload_has_required_fields() {
        let event = PreCompactEvent {
            session_id: "ses_artifact".to_string(),
            cwd: "/tmp/demo".to_string(),
            trigger: "manual".to_string(),
            custom_instructions: None,
            transcript_path: None,
        };
        let signal_summary = BTreeMap::from([("knowledge_exported".to_string(), 1usize)]);
        let payload = compact_summary_artifact_payload(
            &event,
            &["src/lib.rs".to_string(), "src/main.rs".to_string()],
            Some("task-7"),
            &signal_summary,
        );
        let parsed: serde_json::Value = serde_json::from_str(&payload).expect("valid json");

        assert_eq!(parsed["artifact_type"].as_str(), Some("compact_summary"));
        assert_eq!(parsed["session_id"].as_str(), Some("ses_artifact"));
        assert_eq!(parsed["summary_request"].as_str(), Some(SUMMARY_REQUEST));
        assert_eq!(parsed["active_task_id"].as_str(), Some("task-7"));
        assert_eq!(
            parsed["signal_summary"]["knowledge_exported"].as_u64(),
            Some(1)
        );
        assert!(parsed["resume_hint"].is_string());
        assert!(parsed["created_at"].as_u64().is_some());
        let files = parsed["files_modified"].as_array().expect("array");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].as_str(), Some("src/lib.rs"));
    }

    #[test]
    fn dedupes_identical_snapshot_content_within_a_scope() {
        let hash = scope_hash(Some("/tmp/demo"));
        let path = snapshot_capture_state_path(&hash);
        let _ = remove_file_with_lock(&path);

        let content = r#"{"type":"compaction_snapshot","session_id":"abc123","cwd":"/tmp/demo","trigger":"manual","custom_instructions":null,"summary_request":"Please summarize the current work before compaction.","active_errors":[],"files_modified":[],"transcript_path":null}"#;
        assert!(remember_snapshot_capture(&hash, content));
        assert!(!remember_snapshot_capture(&hash, content));

        let stored = load_json_file::<Vec<String>>(&path).unwrap_or_default();
        assert_eq!(stored.len(), 1);

        let _ = remove_file_with_lock(&path);
    }

    #[test]
    fn pre_compact_memory_entries_keeps_only_high_signal_kinds() {
        let outcomes = vec![
            OutcomeEvent::new(OutcomeKind::ErrorDetected, "an error appeared"),
            OutcomeEvent::new(OutcomeKind::ErrorResolved, "fixed the error"),
            OutcomeEvent::new(OutcomeKind::SelfCorrection, "corrected myself"),
            OutcomeEvent::new(OutcomeKind::ValidationPassed, "tests pass"),
            OutcomeEvent::new(OutcomeKind::DocumentIngested, "ingested a doc"),
            OutcomeEvent::new(OutcomeKind::KnowledgeExported, "exported knowledge"),
        ];

        let entries = pre_compact_memory_entries(&outcomes, Some("cortina"));

        // Only ErrorResolved, ValidationPassed, KnowledgeExported survive.
        assert_eq!(entries.len(), 3);
        let topics: Vec<&str> = entries.iter().map(|(topic, _)| topic.as_str()).collect();
        assert!(topics.contains(&"context/cortina/pre-compact/memory/error-resolved"));
        assert!(topics.contains(&"context/cortina/pre-compact/memory/validation-passed"));
        assert!(topics.contains(&"context/cortina/pre-compact/memory/knowledge-exported"));
    }

    #[test]
    fn pre_compact_memory_entries_uses_session_topic_without_project() {
        let outcomes = vec![OutcomeEvent::new(OutcomeKind::ErrorResolved, "fixed it")];

        let entries = pre_compact_memory_entries(&outcomes, None);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "session/pre-compact/memory/error-resolved");
        assert_eq!(entries[0].1, "fixed it");
    }

    #[test]
    fn pre_compact_memory_entries_appends_command_to_content() {
        let mut outcome = OutcomeEvent::new(OutcomeKind::ValidationPassed, "tests pass");
        outcome.command = Some("cargo test".to_string());

        let entries = pre_compact_memory_entries(&[outcome], Some("cortina"));

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "tests pass\ncommand: cargo test");
    }

    #[test]
    fn pre_compact_memory_entries_caps_at_eight_most_recent() {
        let outcomes: Vec<OutcomeEvent> = (0..10)
            .map(|i| OutcomeEvent::new(OutcomeKind::ErrorResolved, format!("e{i}")))
            .collect();

        let entries = pre_compact_memory_entries(&outcomes, Some("cortina"));

        // Bounded to 8, and the most recent (highest index) come first.
        assert_eq!(entries.len(), 8);
        assert_eq!(entries[0].1, "e9");
        assert_eq!(entries[7].1, "e2");
    }

    #[test]
    fn pre_compact_memory_entries_excludes_snapshot_marker_keeps_genuine() {
        // Construct a compaction marker the same way production does — via the
        // `.with_signal_type(...)` builder — so the test exercises the real path.
        let marker = OutcomeEvent::new(
            OutcomeKind::KnowledgeExported,
            "pre-compact snapshot stored in hyphae (context/cortina/pre-compact)",
        )
        .with_signal_type("compaction_marker");

        // Construct a genuine KnowledgeExported outcome (no compaction_marker signal_type).
        let genuine = OutcomeEvent::new(
            OutcomeKind::KnowledgeExported,
            "real knowledge was exported",
        );

        let outcomes = vec![marker, genuine];
        let entries = pre_compact_memory_entries(&outcomes, Some("cortina"));

        // Only the genuine outcome should survive; the marker should be excluded.
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].0,
            "context/cortina/pre-compact/memory/knowledge-exported"
        );
        assert_eq!(entries[0].1, "real knowledge was exported");
    }
}

use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::events::{NormalizedLifecycleEvent, OutcomeEvent, OutcomeKind};
use crate::outcomes::{load_outcomes, record_outcome};
use crate::policy::FAIL_OPEN_LIFECYCLE_CAPTURE;
use crate::utils::{
    Importance, command_exists, current_agent_id_for_cwd, current_task_id_for_cwd,
    current_timestamp_ms, ensure_scoped_hyphae_session, load_json_file, project_name_for_cwd,
    scope_hash, store_compact_summary_artifact, store_in_hyphae, temp_state_path, update_json_file,
};

use super::post_tool_use::{get_pending_documents, get_pending_files};

const MAX_RECORDED_SNAPSHOTS: usize = 16;
const SNAPSHOT_SESSION_TASK: &str = "pre compact snapshot";
const SUMMARY_REQUEST: &str = "Please summarize the current work before compaction.";

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
    let envelope = match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            const { assert!(FAIL_OPEN_LIFECYCLE_CAPTURE) };
            return Ok(());
        }
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

    if command_exists("hyphae") {
        let _ = ensure_scoped_hyphae_session(Some(&event.cwd), Some(SNAPSHOT_SESSION_TASK));
        let project = project_name_for_cwd(Some(&event.cwd));
        let topic = project.as_deref().map_or_else(
            || "session/compaction-snapshot".to_string(),
            |name| format!("context/{name}/pre-compact"),
        );
        let agent_id = current_agent_id_for_cwd(Some(&event.cwd));
        store_in_hyphae(&topic, &content, Importance::High, project.as_deref(), agent_id.as_deref());
        let outcome = OutcomeEvent::new(
            OutcomeKind::KnowledgeExported,
            format!("pre-compact snapshot stored in hyphae ({topic})"),
        );
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
    }
}

fn compaction_snapshot_content(
    event: &crate::events::PreCompactEvent,
    active_errors: &BTreeMap<String, ActiveErrorEntry>,
    files_modified: &[String],
    active_task_id: Option<&str>,
    signal_summary: &BTreeMap<String, usize>,
) -> String {
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
        let signal_summary =
            BTreeMap::from([("knowledge_exported".to_string(), 1usize)]);
        let payload = compact_summary_artifact_payload(
            &event,
            &["src/lib.rs".to_string(), "src/main.rs".to_string()],
            Some("task-7"),
            &signal_summary,
        );
        let parsed: serde_json::Value = serde_json::from_str(&payload).expect("valid json");

        assert_eq!(parsed["artifact_type"].as_str(), Some("compact_summary"));
        assert_eq!(parsed["session_id"].as_str(), Some("ses_artifact"));
        assert_eq!(
            parsed["summary_request"].as_str(),
            Some(SUMMARY_REQUEST)
        );
        assert_eq!(parsed["active_task_id"].as_str(), Some("task-7"));
        assert_eq!(
            parsed["signal_summary"]["knowledge_exported"].as_u64(),
            Some(1)
        );
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
}

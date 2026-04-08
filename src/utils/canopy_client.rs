use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use spore::logging::{SpanContext, subprocess_span, tool_span};
use tracing::{debug, warn};

use crate::events::OutcomeEvent;

use super::hyphae_client::resolved_command;
use super::state::{load_json_file, temp_state_path, update_json_file};

fn span_context(project_root: Option<&str>, tool: &str) -> SpanContext {
    let context = SpanContext::for_app("cortina").with_tool(tool);
    match project_root {
        Some(project_root) if !project_root.trim().is_empty() => {
            context.with_workspace_root(project_root.to_string())
        }
        _ => context,
    }
}

#[derive(Debug, Deserialize)]
struct CanopyAgent {
    project_root: String,
    worktree_id: String,
    current_task_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct EvidenceBridgeStats {
    pub(crate) evidence_refs_written: usize,
    pub(crate) evidence_write_failures: usize,
}

pub(crate) fn evidence_bridge_stats_path(hash: &str) -> PathBuf {
    temp_state_path("evidence-bridge", hash, "json")
}

pub(crate) fn evidence_bridge_stats(hash: &str) -> EvidenceBridgeStats {
    load_json_file(evidence_bridge_stats_path(hash)).unwrap_or_default()
}

pub(crate) fn note_evidence_write_success(hash: &str) {
    let _ =
        update_json_file::<EvidenceBridgeStats, _, _>(evidence_bridge_stats_path(hash), |stats| {
            stats.evidence_refs_written += 1;
        });
}

pub(crate) fn note_evidence_write_failure(hash: &str) {
    let _ =
        update_json_file::<EvidenceBridgeStats, _, _>(evidence_bridge_stats_path(hash), |stats| {
            stats.evidence_write_failures += 1;
        });
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn attach_outcome_evidence(hash: &str, outcome: &OutcomeEvent) {
    let (Some(project_root), Some(worktree_id)) = (
        outcome.project_root.as_deref(),
        outcome.worktree_id.as_deref(),
    ) else {
        return;
    };

    let hash = hash.to_string();
    let outcome = outcome.clone();
    let project_root = project_root.to_string();
    let worktree_id = worktree_id.to_string();

    let _ = thread::Builder::new()
        .name("cortina-evidence-bridge".to_string())
        .spawn(move || {
            let context = span_context(Some(&project_root), "canopy_evidence_bridge");
            let _workflow_span = tool_span("canopy_evidence_bridge", &context).entered();
            let Some(task_id) = active_task_id(&project_root, &worktree_id) else {
                debug!(
                    "No active Canopy task matched project_root/worktree_id for evidence attach"
                );
                return;
            };

            let success = record_outcome_evidence_write(
                || attempt_outcome_evidence_write(&outcome, &task_id),
                thread::sleep,
            );

            if success {
                note_evidence_write_success(&hash);
            } else {
                note_evidence_write_failure(&hash);
                warn!("Canopy evidence write failed after retries for scope {hash}");
                eprintln!("cortina: warn: evidence write failed after retries for scope {hash}");
            }
        });
}

pub(crate) fn record_outcome_evidence_write<F, S>(mut attempt: F, mut sleep: S) -> bool
where
    F: FnMut() -> bool,
    S: FnMut(Duration),
{
    for delay in retry_delays() {
        if attempt() {
            return true;
        }
        sleep(delay);
    }

    attempt()
}

#[cfg_attr(test, allow(dead_code))]
fn active_task_id(project_root: &str, worktree_id: &str) -> Option<String> {
    let context = span_context(Some(project_root), "canopy_agent_list");
    let _tool_span = tool_span("canopy_agent_list", &context).entered();
    let mut command = resolved_command("canopy")?;
    let _subprocess_span = subprocess_span("canopy agent list", &context).entered();
    let output = command.arg("agent").arg("list").output().ok()?;
    if !output.status.success() {
        debug!("canopy agent list returned non-success for worktree {worktree_id}");
        return None;
    }

    parse_active_task_id(&output.stdout, project_root, worktree_id)
}

fn attempt_outcome_evidence_write(outcome: &OutcomeEvent, task_id: &str) -> bool {
    let context = span_context(outcome.project_root.as_deref(), "canopy_evidence_add");
    let _tool_span = tool_span("canopy_evidence_add", &context).entered();
    let Some(mut command) = resolved_command("canopy") else {
        return false;
    };

    for arg in evidence_command_args(task_id, outcome) {
        command.arg(arg);
    }

    let _subprocess_span = subprocess_span("canopy evidence add", &context).entered();
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn parse_active_task_id(payload: &[u8], project_root: &str, worktree_id: &str) -> Option<String> {
    let agents: Vec<CanopyAgent> = serde_json::from_slice(payload).ok()?;
    let task_ids: BTreeSet<_> = agents
        .into_iter()
        .filter(|agent| agent.project_root == project_root && agent.worktree_id == worktree_id)
        .filter_map(|agent| agent.current_task_id)
        .collect();

    (task_ids.len() == 1)
        .then(|| task_ids.into_iter().next())
        .flatten()
}

fn source_ref(outcome: &OutcomeEvent) -> String {
    let session = outcome.session_id.as_deref().unwrap_or("unscoped");
    format!(
        "cortina://outcome/{}/{session}/{}",
        outcome.kind.label(),
        outcome.timestamp
    )
}

fn evidence_command_args(task_id: &str, outcome: &OutcomeEvent) -> Vec<String> {
    let mut args = vec![
        "evidence".to_string(),
        "add".to_string(),
        "--task-id".to_string(),
        task_id.to_string(),
        "--source-kind".to_string(),
        "cortina_event".to_string(),
        "--source-ref".to_string(),
        source_ref(outcome),
        "--label".to_string(),
        outcome.kind.label().to_string(),
    ];

    if !outcome.summary.trim().is_empty() {
        args.push("--summary".to_string());
        args.push(outcome.summary.clone());
    }
    if let Some(session_id) = outcome.session_id.as_deref() {
        args.push("--related-session-id".to_string());
        args.push(session_id.to_string());
    }
    if let Some(file_path) = outcome.file_path.as_deref() {
        args.push("--related-file".to_string());
        args.push(file_path.to_string());
    }

    args
}

fn retry_delays() -> [Duration; 3] {
    [
        Duration::from_millis(100),
        Duration::from_millis(500),
        Duration::from_secs(2),
    ]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        evidence_bridge_stats, evidence_command_args, note_evidence_write_failure,
        note_evidence_write_success, parse_active_task_id, record_outcome_evidence_write,
        source_ref,
    };
    use crate::events::{OutcomeEvent, OutcomeKind};

    #[test]
    fn parse_active_task_id_requires_a_unique_match_for_worktree_identity() {
        let payload = br#"[
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-1"},
          {"project_root":"/repo/demo","worktree_id":"git:beta","current_task_id":"task-2"}
        ]"#;

        assert_eq!(
            parse_active_task_id(payload, "/repo/demo", "git:alpha").as_deref(),
            Some("task-1")
        );
        assert_eq!(
            parse_active_task_id(payload, "/repo/demo", "git:missing"),
            None
        );
    }

    #[test]
    fn parse_active_task_id_rejects_ambiguous_matches() {
        let payload = br#"[
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-1"},
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-2"}
        ]"#;

        assert_eq!(
            parse_active_task_id(payload, "/repo/demo", "git:alpha"),
            None
        );
    }

    #[test]
    fn source_ref_carries_kind_session_and_timestamp() {
        let outcome = OutcomeEvent::new(OutcomeKind::ValidationPassed, "cargo test passed")
            .with_session("ses-1", "demo");

        let reference = source_ref(&outcome);
        assert!(reference.starts_with("cortina://outcome/validation_passed/ses-1/"));
    }

    #[test]
    fn evidence_command_args_use_cortina_event_kind() {
        let outcome = OutcomeEvent::new(OutcomeKind::ErrorDetected, "Command failed")
            .with_session("ses-1", "demo")
            .with_file_path("src/lib.rs");

        let args = evidence_command_args("task-123", &outcome);

        assert!(args.contains(&"evidence".to_string()));
        assert!(args.contains(&"add".to_string()));
        assert!(args.contains(&"--task-id".to_string()));
        assert!(args.contains(&"task-123".to_string()));
        assert!(args.contains(&"--source-kind".to_string()));
        assert!(args.contains(&"cortina_event".to_string()));
        assert!(args.contains(&"--source-ref".to_string()));
        assert!(
            args.iter()
                .any(|arg| arg.starts_with("cortina://outcome/error_detected/ses-1/"))
        );
        assert!(args.contains(&"--related-session-id".to_string()));
        assert!(args.contains(&"ses-1".to_string()));
        assert!(args.contains(&"--related-file".to_string()));
        assert!(args.contains(&"src/lib.rs".to_string()));
    }

    #[test]
    fn record_outcome_evidence_write_retries_and_records_success() {
        let hash = format!("test-evidence-success-{}", std::process::id());
        let mut attempts = 0;
        let mut sleeps = Vec::new();

        let success = record_outcome_evidence_write(
            || {
                attempts += 1;
                attempts >= 3
            },
            |delay| sleeps.push(delay),
        );

        assert!(success);
        assert_eq!(attempts, 3);
        assert_eq!(
            sleeps,
            vec![Duration::from_millis(100), Duration::from_millis(500)]
        );

        note_evidence_write_success(&hash);
        let stats = evidence_bridge_stats(&hash);
        assert_eq!(stats.evidence_refs_written, 1);
        assert_eq!(stats.evidence_write_failures, 0);
    }

    #[test]
    fn record_outcome_evidence_write_records_failure_after_all_retries() {
        let hash = format!("test-evidence-failure-{}", std::process::id());
        let mut attempts = 0;
        let mut sleeps = Vec::new();

        let success = record_outcome_evidence_write(
            || {
                attempts += 1;
                false
            },
            |delay| sleeps.push(delay),
        );

        assert!(!success);
        assert_eq!(attempts, 4);
        assert_eq!(
            sleeps,
            vec![
                Duration::from_millis(100),
                Duration::from_millis(500),
                Duration::from_secs(2),
            ]
        );

        note_evidence_write_failure(&hash);
        let stats = evidence_bridge_stats(&hash);
        assert_eq!(stats.evidence_refs_written, 0);
        assert_eq!(stats.evidence_write_failures, 1);
    }
}

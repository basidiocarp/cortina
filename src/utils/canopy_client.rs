use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use spore::logging::{SpanContext, subprocess_span, tool_span, workflow_span};
use spore::telemetry::TraceContextCarrier;
use tracing::{debug, warn};

use crate::events::OutcomeEvent;
use crate::outcomes::bridge_signals;

use super::hyphae_client::resolved_command;
use super::session_scope::scope_identity_for_cwd;
use super::state::{load_json_file, temp_state_path, update_json_file};

fn span_context(
    project_root: Option<&str>,
    session_id: Option<&str>,
    request_id: Option<&str>,
    tool: &str,
) -> SpanContext {
    let mut context = SpanContext::for_app("cortina").with_tool(tool);
    if let Some(session_id) = session_id {
        context = context.with_session_id(session_id.to_string());
    }
    if let Some(request_id) = request_id {
        context = context.with_request_id(request_id.to_string());
    }
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
    agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EvidenceRefSpec {
    source_ref: String,
    label: String,
    summary: Option<String>,
    related_session_id: Option<String>,
    related_file: Option<String>,
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

    let context = span_context(
        Some(project_root),
        outcome.session_id.as_deref(),
        Some(hash),
        "canopy_evidence_bridge",
    );
    let _workflow_span = workflow_span("canopy_evidence_bridge", &context).entered();
    let Some(task_id) = active_task_id(project_root, worktree_id) else {
        debug!("No active Canopy task matched project_root/worktree_id for evidence attach");
        return;
    };

    for evidence in evidence_specs_for_outcome(outcome) {
        let success = record_outcome_evidence_write(
            || attempt_outcome_evidence_write(&evidence, outcome, &task_id),
            std::thread::sleep,
        );

        if success {
            note_evidence_write_success(hash);
        } else {
            note_evidence_write_failure(hash);
            warn!(
                "Canopy evidence write failed after retries for scope {hash}: {}",
                evidence.label
            );
            eprintln!(
                "cortina: warn: evidence write failed after retries for scope {hash}: {}",
                evidence.label
            );
        }
    }
}

pub(crate) fn current_task_id_for_cwd(cwd: Option<&str>) -> Option<String> {
    let (project_root, worktree_id) = scope_identity_for_cwd(cwd)?;
    active_task_id(&project_root, &worktree_id)
}

pub(crate) fn current_agent_id_for_cwd(cwd: Option<&str>) -> Option<String> {
    let (project_root, worktree_id) = scope_identity_for_cwd(cwd)?;
    active_agent_id(&project_root, &worktree_id)
}

pub(crate) fn record_outcome_evidence_write<F, S>(mut attempt: F, mut sleep: S) -> bool
where
    F: FnMut() -> bool,
    S: FnMut(Duration),
{
    // Keep retries on the calling thread so the CLI waits for the bridge to
    // finish instead of depending on detached work that can be cut off on exit.
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
    let context = span_context(
        Some(project_root),
        None,
        Some(worktree_id),
        "canopy_agent_list",
    );
    let _tool_span = tool_span("canopy_agent_list", &context).entered();
    let mut command = resolved_command("canopy")?;

    if let Some(carrier) = TraceContextCarrier::from_current() {
        command.env("TRACEPARENT", &carrier.traceparent);
        if let Some(ref ts) = carrier.tracestate {
            command.env("TRACESTATE", ts);
        }
    }

    let _subprocess_span = subprocess_span("canopy agent list", &context).entered();
    let output = command.arg("agent").arg("list").output().ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.trim().is_empty() {
            debug!("canopy agent list returned non-success for worktree {worktree_id}");
        } else {
            warn!(
                "canopy agent list returned non-success for worktree {worktree_id}: {}",
                stderr.trim()
            );
            eprint!("{stderr}");
        }
        return None;
    }

    parse_active_task_id(&output.stdout, project_root, worktree_id)
}

fn active_agent_id(project_root: &str, worktree_id: &str) -> Option<String> {
    let context = span_context(
        Some(project_root),
        None,
        Some(worktree_id),
        "canopy_agent_list",
    );
    let _tool_span = tool_span("canopy_agent_list", &context).entered();
    let mut command = resolved_command("canopy")?;

    if let Some(carrier) = TraceContextCarrier::from_current() {
        command.env("TRACEPARENT", &carrier.traceparent);
        if let Some(ref ts) = carrier.tracestate {
            command.env("TRACESTATE", ts);
        }
    }

    let _subprocess_span = subprocess_span("canopy agent list", &context).entered();
    let output = command.arg("agent").arg("list").output().ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.trim().is_empty() {
            debug!("canopy agent list returned non-success for worktree {worktree_id}");
        } else {
            warn!(
                "canopy agent list returned non-success for worktree {worktree_id}: {}",
                stderr.trim()
            );
            eprint!("{stderr}");
        }
        return None;
    }

    parse_active_agent_id(&output.stdout, project_root, worktree_id)
}

fn attempt_outcome_evidence_write(
    evidence: &EvidenceRefSpec,
    outcome: &OutcomeEvent,
    task_id: &str,
) -> bool {
    let context = span_context(
        outcome.project_root.as_deref(),
        outcome.session_id.as_deref(),
        Some(task_id),
        "canopy_evidence_add",
    );
    let _tool_span = tool_span("canopy_evidence_add", &context).entered();
    let Some(mut command) = resolved_command("canopy") else {
        return false;
    };

    if let Some(carrier) = TraceContextCarrier::from_current() {
        command.env("TRACEPARENT", &carrier.traceparent);
        if let Some(ref ts) = carrier.tracestate {
            command.env("TRACESTATE", ts);
        }
    }

    for arg in evidence_command_args(task_id, evidence) {
        command.arg(arg);
    }

    let _subprocess_span = subprocess_span("canopy evidence add", &context).entered();
    let output = match command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            warn!("Failed to execute canopy evidence add: {err}");
            eprintln!("cortina: failed to execute canopy evidence add: {err}");
            return false;
        }
    };

    if output.status.success() {
        true
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.trim().is_empty() {
            warn!("canopy evidence add exited non-zero");
        } else {
            warn!("canopy evidence add exited non-zero: {}", stderr.trim());
            eprint!("{stderr}");
        }
        false
    }
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

fn parse_active_agent_id(payload: &[u8], project_root: &str, worktree_id: &str) -> Option<String> {
    let agents: Vec<CanopyAgent> = serde_json::from_slice(payload).ok()?;
    let agent_ids: BTreeSet<_> = agents
        .into_iter()
        .filter(|agent| agent.project_root == project_root && agent.worktree_id == worktree_id)
        .map(|agent| agent.agent_id)
        .collect();

    (agent_ids.len() == 1)
        .then(|| agent_ids.into_iter().next())
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

fn signal_source_ref(signal_kind: &str, timestamp: u64, session_id: Option<&str>) -> String {
    let session = session_id.unwrap_or("unscoped");
    format!("cortina://signal/{signal_kind}/{session}/{timestamp}")
}

fn describe_causal_signal(
    relation: &str,
    summary: &str,
    signal_kind: &str,
    signal_type: Option<&str>,
) -> String {
    let mut description = summary.trim().to_string();
    if description.is_empty() {
        description = signal_kind.to_string();
    }
    if let Some(signal_type) = signal_type {
        description.push_str(" [");
        description.push_str(signal_type);
        description.push(']');
    }
    if relation.is_empty() {
        description
    } else {
        format!("{relation}: {description}")
    }
}

fn evidence_specs_for_outcome(outcome: &OutcomeEvent) -> Vec<EvidenceRefSpec> {
    let mut specs = Vec::new();
    specs.push(EvidenceRefSpec {
        source_ref: source_ref(outcome),
        label: outcome.kind.label().to_string(),
        summary: (!outcome.summary.trim().is_empty()).then(|| outcome.summary.clone()),
        related_session_id: outcome.session_id.clone(),
        related_file: outcome.file_path.clone(),
    });

    for signal in bridge_signals(outcome) {
        let relation = if outcome.caused_by.as_ref() == Some(&signal) {
            "caused_by"
        } else {
            "signal"
        };
        let signal_label = if relation == "caused_by" {
            format!(
                "caused_by/{}",
                signal.signal_type.as_deref().unwrap_or(&signal.signal_kind)
            )
        } else {
            format!(
                "signal/{}",
                signal.signal_type.as_deref().unwrap_or(&signal.signal_kind)
            )
        };
        let spec = EvidenceRefSpec {
            source_ref: signal_source_ref(
                &signal.signal_kind,
                signal.timestamp,
                signal.session_id.as_deref(),
            ),
            label: signal_label,
            summary: Some(describe_causal_signal(
                relation,
                &signal.summary,
                &signal.signal_kind,
                signal.signal_type.as_deref(),
            )),
            related_session_id: signal.session_id.clone(),
            related_file: signal.file_path.clone(),
        };

        if !specs.iter().any(|existing| existing == &spec) {
            specs.push(spec);
        }
    }

    specs
}

fn evidence_command_args(task_id: &str, evidence: &EvidenceRefSpec) -> Vec<String> {
    let mut args = vec![
        "evidence".to_string(),
        "add".to_string(),
        "--task-id".to_string(),
        task_id.to_string(),
        "--source-kind".to_string(),
        "cortina_event".to_string(),
        "--source-ref".to_string(),
        evidence.source_ref.clone(),
        "--label".to_string(),
        evidence.label.clone(),
    ];

    if let Some(summary) = evidence.summary.as_deref()
        && !summary.trim().is_empty()
    {
        args.push("--summary".to_string());
        args.push(summary.to_string());
    }
    if let Some(session_id) = evidence.related_session_id.as_deref() {
        args.push("--related-session-id".to_string());
        args.push(session_id.to_string());
    }
    if let Some(file_path) = evidence.related_file.as_deref() {
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
        evidence_bridge_stats, evidence_command_args, evidence_specs_for_outcome,
        note_evidence_write_failure, note_evidence_write_success, parse_active_agent_id,
        parse_active_task_id, record_outcome_evidence_write, signal_source_ref, source_ref,
    };
    use crate::events::{CausalSignal, OutcomeEvent, OutcomeKind};

    #[test]
    fn parse_active_task_id_requires_a_unique_match_for_worktree_identity() {
        let payload = br#"[
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-1","agent_id":"agent-1"},
          {"project_root":"/repo/demo","worktree_id":"git:beta","current_task_id":"task-2","agent_id":"agent-2"}
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
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-1","agent_id":"agent-1"},
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-2","agent_id":"agent-1"}
        ]"#;

        assert_eq!(
            parse_active_task_id(payload, "/repo/demo", "git:alpha"),
            None
        );
    }

    #[test]
    fn parse_active_agent_id_requires_a_unique_match_for_worktree_identity() {
        let payload = br#"[
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-1","agent_id":"agent-1"},
          {"project_root":"/repo/demo","worktree_id":"git:beta","current_task_id":"task-2","agent_id":"agent-2"}
        ]"#;

        assert_eq!(
            parse_active_agent_id(payload, "/repo/demo", "git:alpha").as_deref(),
            Some("agent-1")
        );
        assert_eq!(
            parse_active_agent_id(payload, "/repo/demo", "git:missing"),
            None
        );
    }

    #[test]
    fn parse_active_agent_id_rejects_ambiguous_matches() {
        let payload = br#"[
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-1","agent_id":"agent-1"},
          {"project_root":"/repo/demo","worktree_id":"git:alpha","current_task_id":"task-2","agent_id":"agent-2"}
        ]"#;

        assert_eq!(
            parse_active_agent_id(payload, "/repo/demo", "git:alpha"),
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

        let args = evidence_command_args(
            "task-123",
            &super::EvidenceRefSpec {
                source_ref: source_ref(&outcome),
                label: outcome.kind.label().to_string(),
                summary: Some(outcome.summary.clone()),
                related_session_id: outcome.session_id.clone(),
                related_file: outcome.file_path.clone(),
            },
        );

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
    fn evidence_specs_include_signal_and_causal_bridge_refs() {
        let caused_by = CausalSignal::new("error_detected", "Command failed: cargo test", 55)
            .with_command("cargo test")
            .with_signal_type("tool_error");
        let outcome = OutcomeEvent::new(OutcomeKind::ErrorResolved, "Retried after fixing test")
            .with_session("ses-1", "demo")
            .with_file_path("src/lib.rs")
            .with_signal_type("tool_retry")
            .with_caused_by(caused_by);

        let specs = evidence_specs_for_outcome(&outcome);

        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].label, "error_resolved");
        assert_eq!(specs[1].label, "signal/tool_retry");
        assert_eq!(specs[2].label, "caused_by/tool_error");
        assert_eq!(
            specs[1].source_ref,
            signal_source_ref("error_resolved", outcome.timestamp, Some("ses-1"))
        );
        assert_eq!(
            specs[2].source_ref,
            signal_source_ref("error_detected", 55, None)
        );
        assert_eq!(
            specs[2].summary.as_deref(),
            Some("caused_by: Command failed: cargo test [tool_error]")
        );
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

    #[test]
    fn span_context_includes_session_and_request_fields() {
        let context = super::span_context(
            Some("/repo/demo"),
            Some("ses-123"),
            Some("request-456"),
            "canopy_evidence_bridge",
        );

        assert_eq!(context.session_id.as_deref(), Some("ses-123"));
        assert_eq!(context.request_id.as_deref(), Some("request-456"));
        assert_eq!(context.workspace_root.as_deref(), Some("/repo/demo"));
    }
}

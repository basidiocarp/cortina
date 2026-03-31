use std::collections::BTreeSet;
use std::process::Stdio;

use serde::Deserialize;

use crate::events::OutcomeEvent;

use super::hyphae_client::resolved_command;

#[derive(Debug, Deserialize)]
struct CanopyAgent {
    project_root: String,
    worktree_id: String,
    current_task_id: Option<String>,
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn attach_outcome_evidence(outcome: &OutcomeEvent) {
    let (Some(project_root), Some(worktree_id)) = (
        outcome.project_root.as_deref(),
        outcome.worktree_id.as_deref(),
    ) else {
        return;
    };

    let Some(task_id) = active_task_id(project_root, worktree_id) else {
        return;
    };
    let Some(mut command) = resolved_command("canopy") else {
        return;
    };

    command
        .args(["evidence", "add"])
        .args(["--task-id", &task_id])
        .args(["--source-kind", "cortina-event"])
        .args(["--source-ref", &source_ref(outcome)])
        .args(["--label", outcome.kind.label()]);

    if !outcome.summary.trim().is_empty() {
        command.args(["--summary", &outcome.summary]);
    }
    if let Some(session_id) = outcome.session_id.as_deref() {
        command.args(["--related-session-id", session_id]);
    }
    if let Some(file_path) = outcome.file_path.as_deref() {
        command.args(["--related-file", file_path]);
    }

    let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

#[cfg_attr(test, allow(dead_code))]
fn active_task_id(project_root: &str, worktree_id: &str) -> Option<String> {
    let Some(mut command) = resolved_command("canopy") else {
        return None;
    };
    let output = command.arg("agent").arg("list").output().ok()?;
    if !output.status.success() {
        return None;
    }

    parse_active_task_id(&output.stdout, project_root, worktree_id)
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

#[cfg(test)]
mod tests {
    use super::{parse_active_task_id, source_ref};
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
}

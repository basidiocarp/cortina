use std::collections::{BTreeMap, BTreeSet};

use crate::events::{OutcomeEvent, OutcomeKind};
use crate::policy::capture_policy;
use crate::utils::SessionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptSummary {
    pub task_desc: String,
    pub files_modified: Vec<String>,
    pub tool_counts: String,
    pub errors_encountered: usize,
    pub outcome: String,
}

impl Default for TranscriptSummary {
    fn default() -> Self {
        Self {
            task_desc: "Session work".to_string(),
            files_modified: Vec::new(),
            tool_counts: String::new(),
            errors_encountered: 0,
            outcome: "Work completed".to_string(),
        }
    }
}

pub(super) fn merge_structured_outcomes(
    mut summary: TranscriptSummary,
    outcomes: &[OutcomeEvent],
) -> TranscriptSummary {
    if outcomes.is_empty() {
        return summary;
    }

    let mut files: BTreeSet<String> = summary.files_modified.into_iter().collect();
    for outcome in outcomes {
        if let Some(file_path) = outcome.file_path.as_ref().filter(|path| !path.is_empty()) {
            files.insert(file_path.clone());
        }
    }
    summary.files_modified = files.into_iter().collect();

    let structured_error_count = outcomes
        .iter()
        .filter(|event| matches!(event.kind, OutcomeKind::ErrorDetected))
        .count();
    summary.errors_encountered = summary.errors_encountered.max(structured_error_count);

    if (summary.outcome.trim().is_empty() || summary.outcome == "Work completed")
        && let Some(latest) = outcomes.last()
    {
        summary.outcome.clone_from(&latest.summary);
    }

    summary
}

pub(super) fn format_structured_outcome_attribution(outcomes: &[OutcomeEvent]) -> Option<String> {
    if outcomes.is_empty() {
        return None;
    }

    let mut counts: BTreeMap<OutcomeKind, usize> = BTreeMap::new();
    for outcome in outcomes {
        *counts.entry(outcome.kind).or_insert(0) += 1;
    }

    Some(
        counts
            .into_iter()
            .map(|(kind, count)| format!("{}({count})", kind.label()))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub(super) fn filter_outcomes_for_session(
    outcomes: &[OutcomeEvent],
    session: Option<&SessionState>,
    project: &str,
) -> Vec<OutcomeEvent> {
    let Some(session) = session else {
        return outcomes.to_vec();
    };

    let grace_ms = capture_policy().outcome_attribution_grace_ms;

    let session_scoped: Vec<OutcomeEvent> = outcomes
        .iter()
        .filter(|event| event.session_id.as_deref() == Some(session.session_id.as_str()))
        .cloned()
        .collect();

    if let (Some(project_root), Some(worktree_id)) = (
        session.project_root.as_deref(),
        session.worktree_id.as_deref(),
    ) {
        return outcomes
            .iter()
            .filter(|event| {
                event.session_id.as_deref() == Some(session.session_id.as_str())
                    || (event.session_id.is_none()
                        && event.project_root.as_deref() == Some(project_root)
                        && event.worktree_id.as_deref() == Some(worktree_id)
                        && (session.started_at == 0
                            || event.timestamp.saturating_add(grace_ms) >= session.started_at))
            })
            .cloned()
            .collect();
    }

    let unattributed_current_session: Vec<OutcomeEvent> = outcomes
        .iter()
        .filter(|event| {
            event.session_id.is_none()
                && event.project.is_none()
                && event.timestamp.saturating_add(grace_ms) >= session.started_at
        })
        .cloned()
        .collect();

    if !session_scoped.is_empty() || !unattributed_current_session.is_empty() {
        return outcomes
            .iter()
            .filter(|event| {
                event.session_id.as_deref() == Some(session.session_id.as_str())
                    || (event.session_id.is_none()
                        && event.project.is_none()
                        && event.timestamp.saturating_add(grace_ms) >= session.started_at)
            })
            .cloned()
            .collect();
    }

    if outcomes.iter().any(|event| event.session_id.is_some()) {
        return Vec::new();
    }

    let project_scoped: Vec<OutcomeEvent> = outcomes
        .iter()
        .filter(|event| {
            event.project.as_deref() == Some(project)
                && (session.started_at == 0
                    || event.timestamp.saturating_add(grace_ms) >= session.started_at)
        })
        .cloned()
        .collect();
    if !project_scoped.is_empty() {
        return project_scoped;
    }

    outcomes.to_vec()
}

pub(super) fn has_unresolved_errors(outcomes: &[OutcomeEvent]) -> bool {
    outcomes
        .iter()
        .fold(false, |has_unresolved_error, event| match event.kind {
            OutcomeKind::ErrorDetected => true,
            OutcomeKind::ErrorResolved => false,
            _ => has_unresolved_error,
        })
}

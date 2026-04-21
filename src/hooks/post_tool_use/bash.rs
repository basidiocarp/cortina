use std::collections::HashMap;
use std::path::Path;

use crate::events::{BashToolEvent, OutcomeEvent, OutcomeKind};
use crate::outcomes::{error_causal_signal, record_outcome};
use crate::policy::capture_policy;
use crate::utils::{
    Importance, command_exists, current_timestamp_ms, ensure_scoped_hyphae_session, has_error,
    is_build_command, is_significant_command, log_scoped_hyphae_feedback_signal, normalize_command,
    project_name_for_cwd, scope_hash, store_in_hyphae, successful_validation_feedback,
    update_json_file,
};

use super::{annotate_outcome_with_session, pending, truncate};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ErrorEntry {
    command: String,
    error: String,
    timestamp: u64,
    session_id: Option<String>,
    project: Option<String>,
    project_root: Option<String>,
    worktree_id: Option<String>,
}

impl ErrorEntry {
    fn causal_signal(&self) -> crate::events::CausalSignal {
        let mut signal = error_causal_signal(
            &self.command,
            &format!("Command failed: {}", truncate(&self.command, 200)),
            self.timestamp,
            None,
        );
        signal.session_id.clone_from(&self.session_id);
        signal.project.clone_from(&self.project);
        signal.project_root.clone_from(&self.project_root);
        signal.worktree_id.clone_from(&self.worktree_id);
        signal
    }
}

pub(super) fn handle_bash(event: &BashToolEvent) {
    let command = event.command.as_str();
    let output = event.output.as_str();
    let exit_code = event.exit_code;
    let scope_cwd = event.cwd.as_deref();

    if command.is_empty() || !is_significant_command(command) {
        return;
    }

    let session = if command_exists("hyphae") {
        ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200)))
    } else {
        None
    };
    let project = project_name_for_cwd(scope_cwd);

    let hash = scope_hash(scope_cwd);
    let track_file = crate::utils::temp_state_path("errors", &hash, "json");
    let cmd_key = normalize_command(command);
    let error_detected = has_error(output, exit_code);

    if error_detected {
        if track_error(
            &track_file,
            &cmd_key,
            command,
            output,
            session.as_ref(),
            project.clone(),
        ) {
            let outcome = annotate_outcome_with_session(
                session.clone(),
                OutcomeEvent::new(
                    OutcomeKind::ErrorDetected,
                    format!("Command failed: {}", truncate(command, 200)),
                )
                .with_command(truncate(command, 500)),
            );
            let inserted = record_outcome(&hash, outcome);
            if inserted && command_exists("hyphae") {
                store_error_in_hyphae(command, output, scope_cwd);
                log_scoped_hyphae_feedback_signal(
                    scope_cwd,
                    "tool_error",
                    -1,
                    "cortina.post_tool_use.error_detected",
                    Some(&truncate(command, 200)),
                );
            }
        }
    } else {
        resolve_error(&track_file, &cmd_key, command, &hash, scope_cwd);
        log_validation_success(command, exit_code, &hash, scope_cwd);
    }

    if is_build_command(command) && exit_code.is_none_or(|c| c == 0) {
        pending::check_and_trigger_exports(&hash, scope_cwd);
    }
}

pub(super) fn log_validation_success(
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
    if !record_outcome(hash, outcome) {
        return;
    }

    log_scoped_hyphae_feedback_signal(
        scope_cwd,
        signal_type,
        signal_value,
        source,
        Some(&truncate(command, 200)),
    );
}

fn track_error(
    track_file: &Path,
    cmd_key: &str,
    command: &str,
    output: &str,
    session: Option<&crate::utils::SessionState>,
    project: Option<String>,
) -> bool {
    let command = command.chars().take(500).collect::<String>();
    let error = output.chars().take(500).collect::<String>();
    let dedupe_window_ms = capture_policy().outcome_dedupe_window_ms;

    update_json_file::<HashMap<String, ErrorEntry>, _, _>(track_file, |entries| {
        if let Some(existing) = entries.get(cmd_key)
            && existing.command == command
            && existing.error == error
            && current_timestamp_ms().saturating_sub(existing.timestamp) <= dedupe_window_ms
        {
            return false;
        }

        entries.insert(
            cmd_key.to_string(),
            ErrorEntry {
                command,
                error,
                timestamp: current_timestamp_ms(),
                session_id: session.map(|value| value.session_id.clone()),
                project,
                project_root: session.and_then(|value| value.project_root.clone()),
                worktree_id: session.and_then(|value| value.worktree_id.clone()),
            },
        );
        true
    })
    .unwrap_or(false)
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
        let session = ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200)));
        let caused_by = prev_error.causal_signal();
        let outcome = annotate_outcome_with_session(
            session,
            OutcomeEvent::new(
                OutcomeKind::ErrorResolved,
                format!("Recovered command: {}", truncate(command, 200)),
            )
            .with_command(truncate(command, 500))
            .with_signal_type("error_resolved"),
        )
        .with_caused_by(caused_by);
        let inserted = record_outcome(hash, outcome);

        if inserted && command_exists("hyphae") {
            let content = format!(
                "Fixed: {}\nPrevious error: {}",
                truncate(command, 200),
                truncate(&prev_error.error, 300)
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
        truncate(command, 200),
        truncate(output, 500)
    );
    store_in_hyphae(
        "errors/active",
        &content,
        Importance::Medium,
        project_name_for_cwd(scope_cwd).as_deref(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_entry_causal_signal_preserves_original_session_identity() {
        let entry = ErrorEntry {
            command: "cargo test".to_string(),
            error: "failed".to_string(),
            timestamp: 55,
            session_id: Some("ses-original".to_string()),
            project: Some("demo".to_string()),
            project_root: Some("/tmp/demo".to_string()),
            worktree_id: Some("git:demo".to_string()),
        };

        let signal = entry.causal_signal();

        assert_eq!(signal.signal_kind, "error_detected");
        assert_eq!(signal.command.as_deref(), Some("cargo test"));
        assert_eq!(signal.session_id.as_deref(), Some("ses-original"));
        assert_eq!(signal.project.as_deref(), Some("demo"));
        assert_eq!(signal.project_root.as_deref(), Some("/tmp/demo"));
        assert_eq!(signal.worktree_id.as_deref(), Some("git:demo"));
    }
}

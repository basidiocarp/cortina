use std::collections::HashMap;
use std::path::Path;

use crate::events::{BashToolEvent, OutcomeEvent, OutcomeKind};
use crate::outcomes::record_outcome;
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
}

pub(super) fn handle_bash(event: &BashToolEvent) {
    let command = event.command.as_str();
    let output = event.output.as_str();
    let exit_code = event.exit_code;
    let scope_cwd = event.cwd.as_deref();

    if command.is_empty() || !is_significant_command(command) {
        return;
    }

    if command_exists("hyphae") {
        let _ = ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200)));
    }

    let hash = scope_hash(scope_cwd);
    let track_file = crate::utils::temp_state_path("errors", &hash, "json");
    let cmd_key = normalize_command(command);
    let error_detected = has_error(output, exit_code);

    if error_detected {
        track_error(&track_file, &cmd_key, command, output);
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200))),
            OutcomeEvent::new(
                OutcomeKind::ErrorDetected,
                format!("Command failed: {}", truncate(command, 200)),
            )
            .with_command(truncate(command, 500)),
        );
        record_outcome(&hash, outcome);
        if command_exists("hyphae") {
            store_error_in_hyphae(command, output, scope_cwd);
            log_scoped_hyphae_feedback_signal(
                scope_cwd,
                "tool_error",
                -1,
                "cortina.post_tool_use.error_detected",
                Some(&truncate(command, 200)),
            );
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
    record_outcome(hash, outcome);

    log_scoped_hyphae_feedback_signal(
        scope_cwd,
        signal_type,
        signal_value,
        source,
        Some(&truncate(command, 200)),
    );
}

fn track_error(track_file: &Path, cmd_key: &str, command: &str, output: &str) {
    let _ = update_json_file::<HashMap<String, ErrorEntry>, _, _>(track_file, |entries| {
        entries.insert(
            cmd_key.to_string(),
            ErrorEntry {
                command: command.chars().take(500).collect(),
                error: output.chars().take(500).collect(),
                timestamp: current_timestamp_ms(),
            },
        );
    });
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
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(command, 200))),
            OutcomeEvent::new(
                OutcomeKind::ErrorResolved,
                format!("Recovered command: {}", truncate(command, 200)),
            )
            .with_command(truncate(command, 500))
            .with_signal_type("error_resolved"),
        );
        record_outcome(hash, outcome);

        if command_exists("hyphae") {
            let content = format!(
                "Fixed: {}\nPrevious error: {}",
                &command[..command.len().min(200)],
                &prev_error.error[..prev_error.error.len().min(300)]
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
        &command[..command.len().min(200)],
        &output[..output.len().min(500)]
    );
    store_in_hyphae(
        "errors/active",
        &content,
        Importance::Medium,
        project_name_for_cwd(scope_cwd).as_deref(),
    );
}

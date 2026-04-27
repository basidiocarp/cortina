mod bash;
mod edits;
mod pending;
mod secret_redaction;
#[cfg(test)]
mod tests;

use anyhow::Result;
pub use secret_redaction::redact_secrets;

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::events::{OutcomeEvent, ToolResultEvent};
use crate::utils::SessionState;

pub(crate) use pending::{get_pending_documents, get_pending_files};

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> Result<()> {
    let envelope = match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            return Ok(());
        }
    };

    match envelope.tool_result_event() {
        Some(ToolResultEvent::Bash(event)) => bash::handle_bash(&event),
        Some(ToolResultEvent::FileEdit(event)) => edits::handle_file_edits(&event),
        None => {}
    }

    if let Some(tool_name) = envelope.tool_name() {
        let hash = crate::utils::scope_hash(envelope.cwd());
        let source = crate::tool_usage::source_for_tool(tool_name);
        crate::tool_usage::record_tool_call(tool_name, source, &hash);

        // Classify risk and emit as a structured debug event on the lifecycle signal.
        // Cortina is advisory only — it never blocks based on this score.
        let file_path = envelope.tool_input_string("file_path");
        let (risk, level) = crate::risk::classify_tool_call(tool_name, file_path);
        tracing::debug!(
            tool = tool_name,
            risk_score = risk.composite(),
            risk_level = ?level,
            // signal keyword kept for downstream grep / log correlation
            "tool call risk signal emitted"
        );
    }

    print!("{input}");
    Ok(())
}

/// Track file paths extracted from a user prompt into the pending-exports state.
pub(crate) fn track_prompt_file_refs(paths: &[String], hash: &str) {
    for path in paths {
        pending::track_pending_file(path, hash);
    }
}

pub(super) fn truncate(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

pub(super) fn annotate_outcome_with_session(
    session: Option<SessionState>,
    outcome: OutcomeEvent,
) -> OutcomeEvent {
    match session {
        Some(state) => outcome.with_session_state(&state),
        None => outcome,
    }
}

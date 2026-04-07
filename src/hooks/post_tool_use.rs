mod bash;
mod edits;
mod pending;
#[cfg(test)]
mod tests;

use anyhow::Result;

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

    print!("{input}");
    Ok(())
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

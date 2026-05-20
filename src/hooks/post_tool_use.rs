mod bash;
mod edits;
mod memory_store;
mod pending;
mod secret_redaction;
#[cfg(test)]
mod tests;

use anyhow::Result;
pub use secret_redaction::redact_secrets;

use crate::events::{OutcomeEvent, ToolResultEvent};
use crate::utils::SessionState;
use crate::hooks::node_context;
use super::parse_error::parse_or_allow;

pub(crate) use pending::{get_pending_documents, get_pending_files};

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> Result<()> {
    let Some(envelope) = parse_or_allow(input) else { return Ok(()); };

    match envelope.tool_result_event() {
        Some(ToolResultEvent::Bash(event)) => bash::handle_bash(&event),
        Some(ToolResultEvent::FileEdit(event)) => edits::handle_file_edits(&event),
        None => {
            // MultiEdit delivers edits as an array — emit one event per element.
            for event in envelope.multi_edit_events() {
                edits::handle_file_edits(&event);
            }
        }
    }

    // MCP tool handlers — not matched by tool_result_event() above.
    // Minimal-profile suppression is enforced by the adapter env gate before this point.
    if envelope.tool_name_is("hyphae_memory_store") {
        memory_store::validate_memory_topic(&envelope);
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

        // Node context: emit additional_context messages from node-level post_tool_use hooks
        // as a Claude Code additionalContext response (appended to the model's next turn).
        let node_ctx = node_context::load_node_context()
            .unwrap_or_else(|e| { tracing::warn!("Failed to parse CORTINA_NODE_CONTEXT: {e}"); None });

        if let Some(ref ctx) = node_ctx {
            let extras = ctx.post_tool_additional_context(tool_name);
            for msg in &extras {
                tracing::info!(
                    tool = tool_name,
                    node_id = ctx.node_id.as_deref().unwrap_or("unknown"),
                    "[cortina] Node-level additional context: {msg}"
                );
            }
            if let Some(response) = crate::adapters::claude_code::additional_context_response(&extras) {
                println!("{response}");
            }
        }
    }

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

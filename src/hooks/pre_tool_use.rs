use anyhow::Result;
use serde_json::json;
use std::process::Command;

use crate::event_envelope::EventEnvelope;
use crate::utils::command_exists;

/// Handle `PreToolUse` events: rewrite commands through Mycelium.
///
/// Replaces mycelium-rewrite.sh. Reads the tool input, checks if the
/// command should be rewritten via `mycelium rewrite`, and outputs the
/// updated input JSON if a rewrite occurred.
#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> Result<()> {
    let envelope = match EventEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            return Ok(());
        }
    };

    let cmd = match envelope.tool_input_string("command") {
        Some(command) => command.to_string(),
        None => return Ok(()),
    };

    if cmd.is_empty() {
        return Ok(());
    }

    // Skip heredocs — they contain too much complexity
    if cmd.contains("<<") {
        return Ok(());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Check if mycelium is available
    // ─────────────────────────────────────────────────────────────────────────
    if !command_exists("mycelium") {
        return Ok(());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Delegate to mycelium rewrite
    // ─────────────────────────────────────────────────────────────────────────
    let output = Command::new("mycelium").args(["rewrite", &cmd]).output();

    let rewritten = match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_) => return Ok(()), // mycelium rewrite failed, pass through
    };

    // If no change, nothing to do
    if rewritten == cmd {
        return Ok(());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Output rewrite instruction JSON
    // ─────────────────────────────────────────────────────────────────────────
    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "Mycelium auto-rewrite",
            "updatedInput": envelope.updated_input_with_command(&rewritten)
        }
    });

    println!("{response}");
    Ok(())
}

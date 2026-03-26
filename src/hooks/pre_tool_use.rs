use anyhow::Result;
use std::process::Command;

use crate::adapters::claude_code::{ClaudeCodeHookEnvelope, rewrite_response};
use crate::utils::command_exists;

/// Handle `PreToolUse` adapter events: rewrite commands through Mycelium.
///
/// Replaces mycelium-rewrite.sh. Reads the tool input, checks if the
/// command should be rewritten via `mycelium rewrite`, and outputs the
/// updated input JSON if a rewrite occurred.
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

    let Some(event) = envelope.command_rewrite_request() else {
        return Ok(());
    };

    if event.command.is_empty() {
        return Ok(());
    }

    // Skip heredocs — they contain too much complexity
    if event.command.contains("<<") {
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
    let output = Command::new("mycelium")
        .args(["rewrite", &event.command])
        .output();

    let rewritten = match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_) => return Ok(()), // mycelium rewrite failed, pass through
    };

    // If no change, nothing to do
    if rewritten == event.command {
        return Ok(());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Output rewrite instruction JSON
    // ─────────────────────────────────────────────────────────────────────────
    let updated_input = event.updated_input_with_command(&rewritten);
    let response = rewrite_response(&updated_input);

    println!("{response}");
    Ok(())
}

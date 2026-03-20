use anyhow::Result;

/// Handle PreToolUse events: rewrite commands through Mycelium.
///
/// Replaces mycelium-rewrite.sh. Reads the tool input, checks if the
/// command should be rewritten, and outputs the updated input JSON.
pub fn handle(input: &str) -> Result<()> {
    let _json: serde_json::Value = serde_json::from_str(input).unwrap_or_default();

    // TODO: absorb mycelium rewrite logic
    // - parse tool_input.command
    // - classify via registry
    // - output rewrite instruction JSON

    // For now, pass through (no rewrite)
    Ok(())
}

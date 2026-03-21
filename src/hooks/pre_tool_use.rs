use anyhow::Result;
use serde_json::json;
use std::process::Command;

use crate::utils::command_exists;

/// Handle PreToolUse events: rewrite commands through Mycelium.
///
/// Replaces mycelium-rewrite.sh. Reads the tool input, checks if the
/// command should be rewritten via `mycelium rewrite`, and outputs the
/// updated input JSON if a rewrite occurred.
pub fn handle(input: &str) -> Result<()> {
    let json: serde_json::Value = serde_json::from_str(input).unwrap_or_default();

    // ─────────────────────────────────────────────────────────────────────────
    // Extract command from tool_input
    // ─────────────────────────────────────────────────────────────────────────
    let cmd = match json.get("tool_input").and_then(|v| v.get("command")) {
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => return Ok(()),
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
    let output = Command::new("mycelium").args(&["rewrite", &cmd]).output();

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
    let mut updated_input = json.get("tool_input").cloned().unwrap_or_default();
    if let Some(obj) = updated_input.as_object_mut() {
        obj.insert("command".to_string(), json!(rewritten));
    }

    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "Mycelium auto-rewrite",
            "updatedInput": updated_input
        }
    });

    println!("{}", response.to_string());
    Ok(())
}

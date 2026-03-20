use anyhow::Result;

/// Handle PostToolUse events: capture errors, corrections, code changes.
///
/// Replaces capture-errors.js, capture-corrections.js, capture-code-changes.js.
/// Reads the tool result, detects patterns, stores signals in Hyphae.
pub fn handle(input: &str) -> Result<()> {
    let json: serde_json::Value = serde_json::from_str(input).unwrap_or_default();

    let tool_name = json
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match tool_name {
        "Bash" => {
            // TODO: absorb capture-errors.js logic
            // - detect error patterns in output
            // - track active errors
            // - detect resolution (error → success)
            // - store to hyphae (errors/active, errors/resolved)
        }
        "Write" | "Edit" | "MultiEdit" => {
            // TODO: absorb capture-corrections.js logic
            // - track file edits
            // - detect self-corrections (edit after recent write)
            // - store to hyphae (corrections)

            // TODO: absorb capture-code-changes.js logic
            // - track file edit count
            // - trigger rhizome export after 5+ edits + build
            // - trigger hyphae ingest after 3+ doc edits
        }
        _ => {}
    }

    // Pass through the original input (hooks must not modify output)
    print!("{input}");
    Ok(())
}

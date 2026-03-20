use anyhow::Result;

/// Handle Stop events: capture session summary.
///
/// Replaces session-summary.sh. Parses the transcript for task description,
/// files modified, tools used, errors resolved, and outcome.
/// Stores the summary in Hyphae.
pub fn handle(input: &str) -> Result<()> {
    let json: serde_json::Value = serde_json::from_str(input).unwrap_or_default();

    let _session_id = json.get("session_id").and_then(|v| v.as_str());
    let _transcript_path = json.get("transcript_path").and_then(|v| v.as_str());
    let _cwd = json.get("cwd").and_then(|v| v.as_str());

    // TODO: absorb session-summary.sh logic
    // - parse transcript for task, files, tools, errors, outcome
    // - store structured summary in hyphae
    // - call hyphae session-end if available

    Ok(())
}

use super::parse_error::parse_or_allow;
use super::post_compact_marker_path;
use crate::events::PostCompactEvent;
use crate::utils::{scope_hash, update_json_file};
use serde_json::json;

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> anyhow::Result<()> {
    let Some(envelope) = parse_or_allow(input) else {
        return Ok(());
    };

    let Some(event) = envelope.post_compact_event() else {
        return Ok(());
    };

    capture_post_compact(&event);
    Ok(())
}

fn capture_post_compact(event: &PostCompactEvent) {
    tracing::info!(
        session_id = %event.session_id,
        cwd = %event.cwd,
        transcript_path = ?event.transcript_path,
        "cortina: post_compact event received"
    );

    // Drop a marker keyed by this session+worktree scope so the next
    // UserPromptSubmit re-recalls with --post-compaction, re-surfacing memories
    // the compaction dropped from context. Best-effort: never block the tool.
    let hash = scope_hash(Some(&event.cwd));
    let marker_data = json!({ "session_id": event.session_id });
    if let Err(error) =
        update_json_file::<serde_json::Value, _, _>(post_compact_marker_path(&hash), |data| {
            *data = marker_data.clone();
        })
    {
        tracing::warn!(
            ?error,
            "cortina: failed to write post-compaction recall marker"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::remove_file_with_lock;

    #[test]
    fn handle_returns_ok_for_valid_input() {
        // Even with invalid JSON, parse_or_allow returns None and we continue gracefully
        let result = handle("{}");
        assert!(result.is_ok());
    }

    #[test]
    fn handle_returns_ok_for_empty_input() {
        let result = handle("");
        assert!(result.is_ok());
    }

    #[test]
    fn capture_post_compact_writes_marker() {
        let test_cwd = "/tmp/cortina-test-post-compact";
        let hash = scope_hash(Some(test_cwd));
        let marker_path = post_compact_marker_path(&hash);

        // Clean up any pre-existing marker
        let _ = remove_file_with_lock(&marker_path);

        let event = PostCompactEvent {
            session_id: "test-session-123".to_string(),
            cwd: test_cwd.to_string(),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
        };

        capture_post_compact(&event);

        // Verify the marker file was created
        assert!(
            marker_path.exists(),
            "marker file should exist at {marker_path:?}"
        );

        // Clean up
        let _ = remove_file_with_lock(&marker_path);
    }
}

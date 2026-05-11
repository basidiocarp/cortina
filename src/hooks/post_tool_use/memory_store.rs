use crate::adapters::claude_code::ClaudeCodeHookEnvelope;

// Applies only to model-originated `hyphae_memory_store` MCP tool calls.
// Internally-used topics (corrections, errors/active, context/commits, etc.)
// are not routed through PostToolUse and are not subject to this check.
const KNOWN_MODEL_MEMORY_PREFIXES: &[&str] = &[
    "errors/resolved",
    "decisions/",
    // exact string, no subtopic hierarchy — matches hyphae server convention
    "preferences",
    "context/",
    "reviews/",
    "patterns/",
    "lessons/",
];

/// Returns `true` when `topic` matches a known model memory prefix.
///
/// Suppression under `minimal` hook profile is enforced upstream by the
/// adapter env gate (`EnvGate::should_skip_event` → `PostToolUse` returns
/// immediately). This function does not need to check the profile itself.
fn topic_is_known(topic: &str) -> bool {
    KNOWN_MODEL_MEMORY_PREFIXES
        .iter()
        .any(|prefix| topic.starts_with(prefix))
}

/// Validates the topic field of a completed `hyphae_memory_store` call.
/// Emits a structured warning to stderr when the topic does not match any
/// known prefix. Does NOT block storage.
pub(super) fn validate_memory_topic(envelope: &ClaudeCodeHookEnvelope) {
    let topic = match envelope.tool_input_string("topic") {
        Some(t) if !t.is_empty() => t,
        _ => return,
    };

    if !topic_is_known(topic) {
        eprintln!(
            "[cortina] memory-discoverability: topic {:?} does not match any known \
             search prefix. Future recalls may not surface this memory. \
             Known prefixes: {}",
            topic,
            KNOWN_MODEL_MEMORY_PREFIXES.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- predicate unit tests (prove the warning logic) ---

    #[test]
    fn known_prefix_exact_match() {
        assert!(topic_is_known("errors/resolved"));
        assert!(topic_is_known("preferences"));
    }

    #[test]
    fn known_prefix_subtopic_match() {
        assert!(topic_is_known("decisions/my-project"));
        assert!(topic_is_known("context/basidiocarp"));
        assert!(topic_is_known("reviews/pr-42"));
        assert!(topic_is_known("patterns/rust-error-handling"));
        assert!(topic_is_known("lessons/2026-05"));
    }

    #[test]
    fn unknown_topic_is_not_known() {
        assert!(!topic_is_known("my-random-note"));
        assert!(!topic_is_known("temp/scratch"));
        assert!(!topic_is_known("miscellaneous"));
    }

    // --- integration tests (validate_memory_topic uses the predicate) ---

    #[test]
    fn valid_topic_does_not_panic() {
        let envelope = make_envelope("errors/resolved");
        validate_memory_topic(&envelope);
    }

    #[test]
    fn unknown_topic_does_not_panic() {
        // Warning is emitted to stderr; no panic expected.
        let envelope = make_envelope("my-random-note");
        validate_memory_topic(&envelope);
    }

    #[test]
    fn empty_topic_is_skipped() {
        let envelope_json = json!({
            "tool_name": "hyphae_memory_store",
            "tool_input": { "topic": "", "content": "x" }
        });
        let envelope = ClaudeCodeHookEnvelope::parse(&envelope_json.to_string()).unwrap();
        validate_memory_topic(&envelope);
    }

    #[test]
    fn missing_topic_is_skipped() {
        let envelope_json = json!({
            "tool_name": "hyphae_memory_store",
            "tool_input": { "content": "x" }
        });
        let envelope = ClaudeCodeHookEnvelope::parse(&envelope_json.to_string()).unwrap();
        validate_memory_topic(&envelope);
    }

    fn make_envelope(topic: &str) -> ClaudeCodeHookEnvelope {
        let json = json!({
            "tool_name": "hyphae_memory_store",
            "tool_input": { "topic": topic, "content": "test" }
        });
        ClaudeCodeHookEnvelope::parse(&json.to_string()).unwrap()
    }
}

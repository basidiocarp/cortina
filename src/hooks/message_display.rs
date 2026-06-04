use serde_json::json;

use super::parse_error::parse_or_allow;
use crate::events::NormalizedLifecycleEvent;
use crate::utils::{
    Importance, command_exists, current_agent_id_for_cwd, project_name_for_cwd, store_in_hyphae,
};

const MESSAGE_DISPLAY_TOPIC: &str = "session/message-display";

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in adapters/mod.rs"
)]
pub fn handle(input: &str) -> anyhow::Result<()> {
    let Some(envelope) = parse_or_allow(input) else {
        return Ok(());
    };

    let Some(event) = envelope.message_display_event() else {
        // Dispatched only for MessageDisplay, so a missing message_text/session_id/cwd
        // is an envelope anomaly worth a trace — not a silent drop. Still fail-open.
        eprintln!("cortina: message_display envelope missing required fields; skipping capture");
        return Ok(());
    };

    if command_exists("hyphae") {
        let normalized = NormalizedLifecycleEvent::from_message_display(&event);
        let payload = json!({
            "type": "message_display",
            "normalized_lifecycle_event": normalized,
            "session_id": event.session_id,
            "cwd": event.cwd,
            "transcript_path": event.transcript_path,
        })
        .to_string();

        let project = project_name_for_cwd(Some(&event.cwd));
        let agent_id = current_agent_id_for_cwd(Some(&event.cwd));
        store_in_hyphae(
            MESSAGE_DISPLAY_TOPIC,
            &payload,
            Importance::Low,
            project.as_deref(),
            agent_id.as_deref(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{LifecycleCategory, LifecycleStatus, MessageDisplayEvent};

    #[test]
    fn handle_returns_ok_for_malformed_input() {
        let result = handle("not valid json {{{{");
        assert!(
            result.is_ok(),
            "malformed input must not propagate an error"
        );
    }

    #[test]
    fn handle_returns_ok_for_empty_input() {
        let result = handle("");
        assert!(result.is_ok(), "empty input must not propagate an error");
    }

    #[test]
    fn handle_returns_ok_when_message_text_missing() {
        // Valid JSON envelope but no message_text field — message_display_event() returns None.
        let result = handle(r#"{"session_id":"abc","cwd":"/tmp/demo"}"#);
        assert!(
            result.is_ok(),
            "missing message_text must not propagate an error"
        );
    }

    #[test]
    fn from_message_display_produces_message_category() {
        let event = MessageDisplayEvent {
            session_id: "ses_msg".to_string(),
            cwd: "/tmp/demo".to_string(),
            message_text: "hello from the assistant".to_string(),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
        };

        let normalized = NormalizedLifecycleEvent::from_message_display(&event);
        assert_eq!(normalized.category, LifecycleCategory::Message);
        assert_eq!(normalized.status, LifecycleStatus::Captured);
        assert_eq!(normalized.event_name, "message_display");
        assert_eq!(normalized.session_id.as_deref(), Some("ses_msg"));
        assert_eq!(normalized.cwd.as_deref(), Some("/tmp/demo"));
        assert!(normalized.fail_open);
        assert!(
            normalized.summary.contains("message display"),
            "summary should indicate message display, got: {}",
            normalized.summary
        );
    }

    #[test]
    fn from_message_display_serializes_category_as_message() {
        let event = MessageDisplayEvent {
            session_id: "ses_msg".to_string(),
            cwd: "/tmp/demo".to_string(),
            message_text: "test content".to_string(),
            transcript_path: None,
        };

        let normalized = NormalizedLifecycleEvent::from_message_display(&event);
        let serialized = serde_json::to_value(&normalized).expect("valid json");
        assert_eq!(serialized["category"].as_str(), Some("message"));
        assert_eq!(
            serialized["schema_version"].as_str(),
            Some(crate::events::NORMALIZED_LIFECYCLE_EVENT_SCHEMA_VERSION)
        );
    }
}

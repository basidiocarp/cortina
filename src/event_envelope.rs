use serde_json::{Value, json};

/// Shared accessors for the current hook event envelope.
pub struct EventEnvelope {
    raw: Value,
}

impl EventEnvelope {
    pub fn parse(input: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(input).map(|raw| Self { raw })
    }

    pub fn tool_name(&self) -> Option<&str> {
        self.raw.get("tool_name").and_then(Value::as_str)
    }

    pub fn tool_input_string(&self, key: &str) -> Option<&str> {
        self.raw
            .get("tool_input")
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
    }

    pub fn tool_output_string(&self, key: &str) -> Option<&str> {
        self.raw
            .get("tool_output")
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
    }

    pub fn tool_output_exit_code(&self) -> Option<i32> {
        self.raw
            .get("tool_output")
            .and_then(|value| value.get("exit_code"))
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
    }

    pub fn cwd(&self) -> Option<&str> {
        self.raw.get("cwd").and_then(Value::as_str)
    }

    pub fn transcript_path(&self) -> Option<&str> {
        self.raw.get("transcript_path").and_then(Value::as_str)
    }

    pub fn updated_input_with_command(&self, command: &str) -> Value {
        let mut updated_input = self.raw.get("tool_input").cloned().unwrap_or_default();
        if let Some(obj) = updated_input.as_object_mut() {
            obj.insert("command".to_string(), json!(command));
        }
        updated_input
    }
}

#[cfg(test)]
mod tests {
    use super::EventEnvelope;

    #[test]
    fn parses_tool_fields_from_event_envelope() {
        let envelope = EventEnvelope::parse(
            r#"{
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"},
                "tool_output": {"output": "ok", "exit_code": 0}
            }"#,
        )
        .expect("valid envelope");

        assert_eq!(envelope.tool_name(), Some("Bash"));
        assert_eq!(envelope.tool_input_string("command"), Some("cargo test"));
        assert_eq!(envelope.tool_output_string("output"), Some("ok"));
        assert_eq!(envelope.tool_output_exit_code(), Some(0));
    }

    #[test]
    fn updates_tool_input_command_without_mutating_original_shape() {
        let envelope = EventEnvelope::parse(
            r#"{
                "tool_input": {"command": "cargo test", "cwd": "/tmp/demo"}
            }"#,
        )
        .expect("valid envelope");

        let updated = envelope.updated_input_with_command("cargo check");

        assert_eq!(
            updated.get("command").and_then(serde_json::Value::as_str),
            Some("cargo check")
        );
        assert_eq!(
            updated.get("cwd").and_then(serde_json::Value::as_str),
            Some("/tmp/demo")
        );
    }
}

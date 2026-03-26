use serde_json::{Value, json};

/// Shared accessors for the current hook event envelope.
pub struct EventEnvelope {
    raw: Value,
}

pub struct PreToolUseEvent {
    pub command: String,
    updated_input: Value,
}

pub enum PostToolUseEvent {
    Bash(BashToolEvent),
    FileEdit(FileEditEvent),
}

pub struct BashToolEvent {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
}

pub struct FileEditEvent {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
}

pub struct StopEvent {
    pub cwd: String,
    pub transcript_path: Option<String>,
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

    pub fn pre_tool_use_event(&self) -> Option<PreToolUseEvent> {
        let command = self.tool_input_string("command")?.to_string();
        Some(PreToolUseEvent {
            updated_input: self.raw.get("tool_input").cloned().unwrap_or_default(),
            command,
        })
    }

    pub fn post_tool_use_event(&self) -> Option<PostToolUseEvent> {
        match self.tool_name()? {
            "Bash" => Some(PostToolUseEvent::Bash(BashToolEvent {
                command: self
                    .tool_input_string("command")
                    .unwrap_or_default()
                    .to_string(),
                output: self
                    .tool_output_string("output")
                    .unwrap_or_default()
                    .to_string(),
                exit_code: self.tool_output_exit_code(),
            })),
            "Write" | "Edit" | "MultiEdit" => Some(PostToolUseEvent::FileEdit(FileEditEvent {
                file_path: self
                    .tool_input_string("file_path")
                    .unwrap_or_default()
                    .to_string(),
                old_string: self
                    .tool_input_string("old_string")
                    .unwrap_or_default()
                    .to_string(),
                new_string: self
                    .tool_input_string("new_string")
                    .unwrap_or_default()
                    .to_string(),
            })),
            _ => None,
        }
    }

    pub fn stop_event(&self) -> Option<StopEvent> {
        let cwd = self.cwd()?.to_string();
        Some(StopEvent {
            cwd,
            transcript_path: self.transcript_path().map(ToString::to_string),
        })
    }
}

impl PreToolUseEvent {
    pub fn updated_input_with_command(&self, command: &str) -> Value {
        let mut updated_input = self.updated_input.clone();
        if let Some(obj) = updated_input.as_object_mut() {
            obj.insert("command".to_string(), json!(command));
        }
        updated_input
    }
}

#[cfg(test)]
mod tests {
    use super::{EventEnvelope, PostToolUseEvent};

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

        let event = envelope
            .pre_tool_use_event()
            .expect("expected pre-tool-use event");
        let updated = event.updated_input_with_command("cargo check");

        assert_eq!(
            updated.get("command").and_then(serde_json::Value::as_str),
            Some("cargo check")
        );
        assert_eq!(
            updated.get("cwd").and_then(serde_json::Value::as_str),
            Some("/tmp/demo")
        );
    }

    #[test]
    fn parses_bash_post_tool_event() {
        let envelope = EventEnvelope::parse(
            r#"{
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"},
                "tool_output": {"output": "ok", "exit_code": 0}
            }"#,
        )
        .expect("valid envelope");

        match envelope.post_tool_use_event() {
            Some(PostToolUseEvent::Bash(event)) => {
                assert_eq!(event.command, "cargo test");
                assert_eq!(event.output, "ok");
                assert_eq!(event.exit_code, Some(0));
            }
            _ => panic!("expected bash tool event"),
        }
    }

    #[test]
    fn parses_stop_event() {
        let envelope = EventEnvelope::parse(
            r#"{
                "cwd": "/tmp/demo",
                "transcript_path": "/tmp/transcript.jsonl"
            }"#,
        )
        .expect("valid envelope");

        let stop = envelope.stop_event().expect("expected stop event");
        assert_eq!(stop.cwd, "/tmp/demo");
        assert_eq!(
            stop.transcript_path.as_deref(),
            Some("/tmp/transcript.jsonl")
        );
    }
}

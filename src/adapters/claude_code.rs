use serde_json::{Value, json};

use crate::events::{
    BashToolEvent, CommandRewriteRequest, FileEditEvent, SessionStopEvent, ToolResultEvent,
};

/// Adapter for the current Claude Code hook-event envelope.
pub struct ClaudeCodeHookEnvelope {
    raw: Value,
}

impl ClaudeCodeHookEnvelope {
    pub fn parse(input: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(input).map(|raw| Self { raw })
    }

    pub fn command_rewrite_request(&self) -> Option<CommandRewriteRequest> {
        if self.tool_name()? != "Bash" {
            return None;
        }
        let command = self.tool_input_string("command")?.to_string();
        Some(CommandRewriteRequest::new(
            command,
            self.raw.get("tool_input").cloned().unwrap_or_default(),
        ))
    }

    pub fn tool_result_event(&self) -> Option<ToolResultEvent> {
        match self.tool_name()? {
            "Bash" => Some(ToolResultEvent::Bash(BashToolEvent {
                command: self
                    .tool_input_string("command")
                    .unwrap_or_default()
                    .to_string(),
                output: self
                    .tool_output_string("output")
                    .unwrap_or_default()
                    .to_string(),
                exit_code: self.tool_output_exit_code(),
                cwd: self.cwd().map(ToString::to_string),
            })),
            "Write" | "Edit" | "MultiEdit" => Some(ToolResultEvent::FileEdit(FileEditEvent {
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
                cwd: self.cwd().map(ToString::to_string),
            })),
            _ => None,
        }
    }

    pub fn session_stop_event(&self) -> Option<SessionStopEvent> {
        let cwd = self.cwd()?.to_string();
        Some(SessionStopEvent {
            cwd,
            transcript_path: self.transcript_path().map(ToString::to_string),
        })
    }

    fn tool_name(&self) -> Option<&str> {
        self.raw.get("tool_name").and_then(Value::as_str)
    }

    fn tool_input_string(&self, key: &str) -> Option<&str> {
        self.raw
            .get("tool_input")
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
    }

    fn tool_output_string(&self, key: &str) -> Option<&str> {
        self.raw
            .get("tool_output")
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
    }

    fn tool_output_exit_code(&self) -> Option<i32> {
        self.raw
            .get("tool_output")
            .and_then(|value| value.get("exit_code"))
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
    }

    fn cwd(&self) -> Option<&str> {
        self.raw.get("cwd").and_then(Value::as_str)
    }

    fn transcript_path(&self) -> Option<&str> {
        self.raw.get("transcript_path").and_then(Value::as_str)
    }
}

pub fn rewrite_response(updated_input: &Value) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "Mycelium auto-rewrite",
            "updatedInput": updated_input
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::events::ToolResultEvent;

    use super::{ClaudeCodeHookEnvelope, rewrite_response};

    #[test]
    fn parses_tool_fields_from_claude_code_envelope() {
        let envelope = ClaudeCodeHookEnvelope::parse(
            r#"{
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"},
                "tool_output": {"output": "ok", "exit_code": 0}
            }"#,
        )
        .expect("valid envelope");

        match envelope.tool_result_event() {
            Some(ToolResultEvent::Bash(event)) => {
                assert_eq!(event.command, "cargo test");
                assert_eq!(event.output, "ok");
                assert_eq!(event.exit_code, Some(0));
                assert_eq!(event.cwd, None);
            }
            _ => panic!("expected bash tool event"),
        }
    }

    #[test]
    fn updates_tool_input_command_without_mutating_original_shape() {
        let envelope = ClaudeCodeHookEnvelope::parse(
            r#"{
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test", "cwd": "/tmp/demo"}
            }"#,
        )
        .expect("valid envelope");

        let event = envelope
            .command_rewrite_request()
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
    fn parses_stop_event() {
        let envelope = ClaudeCodeHookEnvelope::parse(
            r#"{
                "cwd": "/tmp/demo",
                "transcript_path": "/tmp/transcript.jsonl"
            }"#,
        )
        .expect("valid envelope");

        let stop = envelope.session_stop_event().expect("expected stop event");
        assert_eq!(stop.cwd, "/tmp/demo");
        assert_eq!(
            stop.transcript_path.as_deref(),
            Some("/tmp/transcript.jsonl")
        );
    }

    #[test]
    fn builds_claude_rewrite_response() {
        let response = rewrite_response(&json!({
            "command": "cargo check",
            "cwd": "/tmp/demo"
        }));

        assert_eq!(
            response
                .get("hookSpecificOutput")
                .and_then(|value| value.get("hookEventName"))
                .and_then(serde_json::Value::as_str),
            Some("PreToolUse")
        );
        assert_eq!(
            response
                .get("hookSpecificOutput")
                .and_then(|value| value.get("updatedInput"))
                .and_then(|value| value.get("command"))
                .and_then(serde_json::Value::as_str),
            Some("cargo check")
        );
    }

    #[test]
    fn ignores_non_bash_rewrite_requests() {
        let envelope = ClaudeCodeHookEnvelope::parse(
            r#"{
                "tool_name": "SomeFutureTool",
                "tool_input": {"command": "git status", "cwd": "/tmp/demo"}
            }"#,
        )
        .expect("valid envelope");

        assert!(envelope.command_rewrite_request().is_none());
    }
}

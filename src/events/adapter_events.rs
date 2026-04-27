use serde_json::Value;

pub struct CommandRewriteRequest {
    pub command: String,
    updated_input: Value,
}

pub enum ToolResultEvent {
    Bash(BashToolEvent),
    FileEdit(FileEditEvent),
}

pub struct BashToolEvent {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub cwd: Option<String>,
}

pub struct FileEditEvent {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    pub cwd: Option<String>,
}

pub struct SessionStopEvent {
    pub cwd: String,
    pub transcript_path: Option<String>,
}

pub struct UserPromptSubmitEvent {
    pub session_id: String,
    pub cwd: String,
    pub prompt: String,
    pub transcript_path: Option<String>,
}

pub struct PreCompactEvent {
    pub session_id: String,
    pub cwd: String,
    pub trigger: String,
    pub custom_instructions: Option<String>,
    pub transcript_path: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VolvaHookPhase {
    SessionStart,
    BeforePromptSend,
    ResponseComplete,
    BackendFailed,
    SessionEnd,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VolvaBackendKind {
    OfficialCli,
    AnthropicApi,
}

/// Subset of the Volva `ExecutionSessionIdentity` needed for replay identity.
/// Unknown fields are silently ignored (serde default — no `deny_unknown_fields`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VolvaExecutionSession {
    pub session_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VolvaHookEvent {
    pub schema_version: String,
    pub phase: VolvaHookPhase,
    pub backend_kind: VolvaBackendKind,
    pub cwd: String,
    pub prompt_text: String,
    pub prompt_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_session: Option<VolvaExecutionSession>,
}

impl CommandRewriteRequest {
    pub(crate) fn new(command: String, updated_input: Value) -> Self {
        Self {
            command,
            updated_input,
        }
    }

    pub fn updated_input_with_command(&self, command: &str) -> Value {
        let mut updated_input = self.updated_input.clone();
        if let Some(obj) = updated_input.as_object_mut() {
            obj.insert("command".to_string(), serde_json::json!(command));
        }
        updated_input
    }
}

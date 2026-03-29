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

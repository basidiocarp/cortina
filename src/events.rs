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

#[derive(
    Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    ErrorDetected,
    ErrorResolved,
    SelfCorrection,
    ValidationPassed,
    KnowledgeExported,
    DocumentIngested,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct OutcomeEvent {
    pub kind: OutcomeKind,
    pub summary: String,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_type: Option<String>,
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

impl OutcomeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::ErrorDetected => "error_detected",
            Self::ErrorResolved => "error_resolved",
            Self::SelfCorrection => "self_correction",
            Self::ValidationPassed => "validation_passed",
            Self::KnowledgeExported => "knowledge_exported",
            Self::DocumentIngested => "document_ingested",
        }
    }
}

impl OutcomeEvent {
    pub fn new(kind: OutcomeKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
            timestamp: crate::utils::current_timestamp_ms(),
            session_id: None,
            project: None,
            command: None,
            file_path: None,
            signal_type: None,
        }
    }

    pub fn with_session(
        mut self,
        session_id: impl Into<String>,
        project: impl Into<String>,
    ) -> Self {
        self.session_id = Some(session_id.into());
        self.project = Some(project.into());
        self
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    pub fn with_file_path(mut self, file_path: impl Into<String>) -> Self {
        self.file_path = Some(file_path.into());
        self
    }

    pub fn with_signal_type(mut self, signal_type: impl Into<String>) -> Self {
        self.signal_type = Some(signal_type.into());
        self
    }
}

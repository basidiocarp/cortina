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

    pub fn semantically_matches(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.summary == other.summary
            && self.session_id == other.session_id
            && self.project == other.project
            && self.command == other.command
            && self.file_path == other.file_path
            && self.signal_type == other.signal_type
    }
}

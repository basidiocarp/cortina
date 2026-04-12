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
pub struct CausalSignal {
    pub signal_kind: String,
    pub summary: String,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_type: Option<String>,
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
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<CausalSignal>,
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

impl CausalSignal {
    pub fn new(signal_kind: impl Into<String>, summary: impl Into<String>, timestamp: u64) -> Self {
        Self {
            signal_kind: signal_kind.into(),
            summary: summary.into(),
            timestamp,
            session_id: None,
            project: None,
            project_root: None,
            worktree_id: None,
            command: None,
            file_path: None,
            signal_type: None,
        }
    }

    #[allow(dead_code, reason = "Convenience constructor for later bridge work")]
    pub fn from_outcome(outcome: &OutcomeEvent) -> Self {
        let mut caused_by = Self::new(
            outcome.kind.label(),
            outcome.summary.clone(),
            outcome.timestamp,
        );
        caused_by.session_id.clone_from(&outcome.session_id);
        caused_by.project.clone_from(&outcome.project);
        caused_by.project_root.clone_from(&outcome.project_root);
        caused_by.worktree_id.clone_from(&outcome.worktree_id);
        caused_by.command.clone_from(&outcome.command);
        caused_by.file_path.clone_from(&outcome.file_path);
        caused_by.signal_type.clone_from(&outcome.signal_type);
        caused_by
    }

    pub fn with_session_state(mut self, session: &crate::utils::SessionState) -> Self {
        self.session_id = Some(session.session_id.clone());
        self.project = Some(session.project.clone());
        self.project_root.clone_from(&session.project_root);
        self.worktree_id.clone_from(&session.worktree_id);
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

impl OutcomeEvent {
    pub fn new(kind: OutcomeKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
            timestamp: crate::utils::current_timestamp_ms(),
            session_id: None,
            project: None,
            project_root: None,
            worktree_id: None,
            command: None,
            file_path: None,
            signal_type: None,
            caused_by: None,
        }
    }

    #[allow(
        dead_code,
        reason = "Legacy session annotation helper kept for compatibility"
    )]
    pub fn with_session(
        mut self,
        session_id: impl Into<String>,
        project: impl Into<String>,
    ) -> Self {
        self.session_id = Some(session_id.into());
        self.project = Some(project.into());
        self
    }

    pub fn with_session_state(mut self, session: &crate::utils::SessionState) -> Self {
        self.session_id = Some(session.session_id.clone());
        self.project = Some(session.project.clone());
        self.project_root.clone_from(&session.project_root);
        self.worktree_id.clone_from(&session.worktree_id);
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

    pub fn with_caused_by(mut self, caused_by: CausalSignal) -> Self {
        self.caused_by = Some(caused_by);
        self
    }

    pub fn semantically_matches(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.summary == other.summary
            && self.session_id == other.session_id
            && self.project == other.project
            && self.project_root == other.project_root
            && self.worktree_id == other.worktree_id
            && self.command == other.command
            && self.file_path == other.file_path
            && self.signal_type == other.signal_type
            && self.caused_by == other.caused_by
    }
}

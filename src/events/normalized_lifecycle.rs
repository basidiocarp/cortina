use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{BashToolEvent, FileEditEvent, PreCompactEvent, UserPromptSubmitEvent, VolvaHookEvent};

pub const NORMALIZED_LIFECYCLE_EVENT_SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleCategory {
    Host,
    Tool,
    Session,
    Compaction,
    Council,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    Requested,
    Captured,
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHost {
    ClaudeCode,
    Volva,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct NormalizedLifecycleEvent {
    pub schema_version: &'static str,
    pub category: LifecycleCategory,
    pub status: LifecycleStatus,
    pub host: LifecycleHost,
    pub event_name: String,
    pub summary: String,
    pub fail_open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl NormalizedLifecycleEvent {
    fn new(
        category: LifecycleCategory,
        status: LifecycleStatus,
        host: LifecycleHost,
        event_name: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: NORMALIZED_LIFECYCLE_EVENT_SCHEMA_VERSION,
            category,
            status,
            host,
            event_name: event_name.into(),
            summary: summary.into(),
            fail_open: crate::policy::FAIL_OPEN_LIFECYCLE_CAPTURE,
            session_id: None,
            cwd: None,
            project_root: None,
            worktree_id: None,
            tool_name: None,
            trigger: None,
            metadata: BTreeMap::new(),
        }
    }

    #[allow(
        dead_code,
        reason = "Vocabulary constructors are staged ahead of broader lifecycle capture adoption"
    )]
    pub fn from_bash_tool(event: &BashToolEvent) -> Self {
        let status = match event.exit_code {
            Some(0) => LifecycleStatus::Completed,
            Some(_) => LifecycleStatus::Failed,
            None => LifecycleStatus::Captured,
        };
        let summary = format!("bash tool lifecycle captured for `{}`", event.command);
        let mut normalized = Self::new(
            LifecycleCategory::Tool,
            status,
            LifecycleHost::ClaudeCode,
            "bash_tool_result",
            summary,
        );
        normalized.cwd.clone_from(&event.cwd);
        normalized.tool_name = Some("bash".to_string());
        normalized
            .metadata
            .insert("command".to_string(), json!(event.command));
        normalized
            .metadata
            .insert("exit_code".to_string(), json!(event.exit_code));
        normalized
    }

    #[allow(
        dead_code,
        reason = "Vocabulary constructors are staged ahead of broader lifecycle capture adoption"
    )]
    pub fn from_file_edit(event: &FileEditEvent) -> Self {
        let mut normalized = Self::new(
            LifecycleCategory::Tool,
            LifecycleStatus::Captured,
            LifecycleHost::ClaudeCode,
            "file_edit",
            format!("file edit lifecycle captured for `{}`", event.file_path),
        );
        normalized.cwd.clone_from(&event.cwd);
        normalized.tool_name = Some("file_edit".to_string());
        normalized
            .metadata
            .insert("file_path".to_string(), json!(event.file_path));
        normalized
    }

    pub fn from_pre_compact(event: &PreCompactEvent) -> Self {
        let mut normalized = Self::new(
            LifecycleCategory::Compaction,
            LifecycleStatus::Captured,
            LifecycleHost::ClaudeCode,
            "pre_compact",
            "compaction lifecycle captured".to_string(),
        );
        normalized.session_id = Some(event.session_id.clone());
        normalized.cwd = Some(event.cwd.clone());
        normalized.trigger = Some(event.trigger.clone());
        normalized.metadata.insert(
            "custom_instructions".to_string(),
            json!(event.custom_instructions),
        );
        normalized
            .metadata
            .insert("transcript_path".to_string(), json!(event.transcript_path));
        normalized
    }

    pub fn from_volva_hook(event: &VolvaHookEvent) -> Self {
        let status = match event.phase {
            super::VolvaHookPhase::SessionStart => LifecycleStatus::Started,
            super::VolvaHookPhase::BeforePromptSend => LifecycleStatus::Requested,
            super::VolvaHookPhase::ResponseComplete | super::VolvaHookPhase::SessionEnd => {
                LifecycleStatus::Completed
            }
            super::VolvaHookPhase::BackendFailed => LifecycleStatus::Failed,
        };
        let mut normalized = Self::new(
            LifecycleCategory::Host,
            status,
            LifecycleHost::Volva,
            "volva_hook_event",
            format!(
                "volva hook lifecycle captured for phase `{}`",
                phase_label(event)
            ),
        );
        normalized.cwd = Some(event.cwd.clone());
        normalized
            .metadata
            .insert("phase".to_string(), json!(event.phase));
        normalized
            .metadata
            .insert("backend_kind".to_string(), json!(event.backend_kind));
        normalized
            .metadata
            .insert("prompt_summary".to_string(), json!(event.prompt_summary));
        normalized
            .metadata
            .insert("exit_code".to_string(), json!(event.exit_code));
        normalized
    }

    pub fn from_council_prompt(event: &UserPromptSubmitEvent) -> Self {
        let mut normalized = Self::new(
            LifecycleCategory::Council,
            LifecycleStatus::Captured,
            LifecycleHost::ClaudeCode,
            "user_prompt_submit",
            "council lifecycle captured from prompt".to_string(),
        );
        normalized.session_id = Some(event.session_id.clone());
        normalized.cwd = Some(event.cwd.clone());
        normalized.metadata.insert(
            "prompt_excerpt".to_string(),
            json!(prompt_excerpt(&event.prompt)),
        );
        normalized
            .metadata
            .insert("transcript_path".to_string(), json!(event.transcript_path));
        normalized
    }
}

pub fn is_council_prompt(prompt: &str) -> bool {
    let normalized = prompt.trim().to_ascii_lowercase();
    normalized.contains("/council")
        || normalized.contains("task-linked council")
        || normalized.starts_with("council ")
        || normalized.contains(" council ")
}

fn prompt_excerpt(prompt: &str) -> String {
    let trimmed = prompt.trim();
    let mut chars = trimmed.chars();
    let excerpt: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        format!("{excerpt}...")
    } else {
        excerpt
    }
}

fn phase_label(event: &VolvaHookEvent) -> &'static str {
    match event.phase {
        super::VolvaHookPhase::SessionStart => "session_start",
        super::VolvaHookPhase::BeforePromptSend => "before_prompt_send",
        super::VolvaHookPhase::ResponseComplete => "response_complete",
        super::VolvaHookPhase::BackendFailed => "backend_failed",
        super::VolvaHookPhase::SessionEnd => "session_end",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{VolvaBackendKind, VolvaHookPhase};

    #[test]
    fn recognizes_council_prompts() {
        assert!(is_council_prompt("/council review these findings"));
        assert!(is_council_prompt("task-linked council for this worktree"));
        assert!(!is_council_prompt("summarize this repository"));
    }

    #[test]
    fn builds_compaction_lifecycle_event() {
        let event = PreCompactEvent {
            session_id: "ses_123".to_string(),
            cwd: "/tmp/demo".to_string(),
            trigger: "manual".to_string(),
            custom_instructions: Some("summarize state".to_string()),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
        };

        let normalized = NormalizedLifecycleEvent::from_pre_compact(&event);
        assert_eq!(normalized.category, LifecycleCategory::Compaction);
        assert_eq!(normalized.status, LifecycleStatus::Captured);
        assert_eq!(normalized.host, LifecycleHost::ClaudeCode);
        assert_eq!(normalized.session_id.as_deref(), Some("ses_123"));
        assert_eq!(normalized.trigger.as_deref(), Some("manual"));
        assert!(normalized.fail_open);
    }

    #[test]
    fn builds_volva_hook_lifecycle_event() {
        let event = VolvaHookEvent {
            schema_version: "1.0".to_string(),
            phase: VolvaHookPhase::BackendFailed,
            backend_kind: VolvaBackendKind::OfficialCli,
            cwd: "/tmp/demo".to_string(),
            prompt_text: "prompt".to_string(),
            prompt_summary: "prompt".to_string(),
            stdout: None,
            stderr: Some("oops".to_string()),
            exit_code: Some(1),
            error: Some("backend error".to_string()),
        };

        let normalized = NormalizedLifecycleEvent::from_volva_hook(&event);
        assert_eq!(normalized.category, LifecycleCategory::Host);
        assert_eq!(normalized.status, LifecycleStatus::Failed);
        assert_eq!(normalized.host, LifecycleHost::Volva);
        assert_eq!(normalized.cwd.as_deref(), Some("/tmp/demo"));
    }
}

use serde_json::json;
use std::path::PathBuf;

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::events::UserPromptSubmitEvent;
#[cfg(test)]
use crate::utils::load_json_file;
use crate::utils::{
    Importance, command_exists, ensure_scoped_hyphae_session, project_name_for_cwd, scope_hash,
    store_in_hyphae, temp_state_path, update_json_file,
};

const MAX_RECORDED_PROMPTS: usize = 32;
const PROMPT_TOPIC: &str = "session/prompts";
const PROMPT_SESSION_TASK: &str = "user prompt submit";

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> anyhow::Result<()> {
    let envelope = match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            return Ok(());
        }
    };

    let Some(event) = envelope.user_prompt_submit_event() else {
        return Ok(());
    };

    capture_prompt_submit(&event);
    Ok(())
}

fn capture_prompt_submit(event: &UserPromptSubmitEvent) {
    if event.prompt.trim().is_empty() {
        return;
    }

    let hash = scope_hash(Some(&event.cwd));
    let content = prompt_memory_content(event);
    if !remember_prompt_capture(&hash, &content) {
        return;
    }

    if command_exists("hyphae") {
        let _ = ensure_scoped_hyphae_session(Some(&event.cwd), Some(PROMPT_SESSION_TASK));
        let project = project_name_for_cwd(Some(&event.cwd));
        store_in_hyphae(
            PROMPT_TOPIC,
            &content,
            Importance::Medium,
            project.as_deref(),
        );
    }
}

fn prompt_memory_content(event: &UserPromptSubmitEvent) -> String {
    json!({
        "type": "prompt",
        "content": event.prompt,
        "session_id": event.session_id,
        "cwd": event.cwd,
        "transcript_path": event.transcript_path,
    })
    .to_string()
}

fn prompt_capture_state_path(hash: &str) -> PathBuf {
    temp_state_path("prompt-captures", hash, "json")
}

fn remember_prompt_capture(hash: &str, content: &str) -> bool {
    update_json_file::<Vec<String>, _, _>(&prompt_capture_state_path(hash), |captures| {
        if captures.iter().any(|existing| existing == content) {
            return false;
        }

        captures.push(content.to_string());
        if captures.len() > MAX_RECORDED_PROMPTS {
            let overflow = captures.len().saturating_sub(MAX_RECORDED_PROMPTS);
            captures.drain(0..overflow);
        }
        true
    })
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::{remove_file_with_lock, scope_hash};

    #[test]
    fn builds_prompt_memory_content() {
        let event = UserPromptSubmitEvent {
            session_id: "abc123".to_string(),
            cwd: "/tmp/demo".to_string(),
            prompt: "capture this prompt".to_string(),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
        };

        let content = prompt_memory_content(&event);
        assert!(content.contains(r#""type":"prompt""#));
        assert!(content.contains(r#""session_id":"abc123""#));
        assert!(content.contains(r#""content":"capture this prompt""#));
    }

    #[test]
    fn dedupes_identical_prompt_memory_content_within_a_scope() {
        let hash = scope_hash(Some("/tmp/demo"));
        let path = prompt_capture_state_path(&hash);
        let _ = remove_file_with_lock(&path);

        let content = r#"{"type":"prompt","content":"hello","session_id":"abc123","cwd":"/tmp/demo","transcript_path":null}"#;
        assert!(remember_prompt_capture(&hash, content));
        assert!(!remember_prompt_capture(&hash, content));

        let stored = load_json_file::<Vec<String>>(&path).unwrap_or_default();
        assert_eq!(stored.len(), 1);

        let _ = remove_file_with_lock(&path);
    }
}

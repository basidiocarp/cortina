use serde_json::json;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::events::{
    NormalizedLifecycleEvent, OutcomeEvent, OutcomeKind, UserPromptSubmitEvent, is_council_prompt,
};
use crate::outcomes::record_outcome;
use crate::policy::FAIL_OPEN_LIFECYCLE_CAPTURE;
#[cfg(test)]
use crate::utils::load_json_file;
use crate::utils::{
    Importance, command_exists, current_task_id_for_cwd, ensure_scoped_hyphae_session,
    project_name_for_cwd, scope_hash, scope_identity_for_cwd, store_in_hyphae, temp_state_path,
    update_json_file,
};

const MAX_RECORDED_PROMPTS: usize = 32;
const MAX_RECORDED_COUNCIL_EVENTS: usize = 16;
const PROMPT_TOPIC: &str = "session/prompts";
const COUNCIL_TOPIC: &str = "session/council-lifecycle";
const PROMPT_SESSION_TASK: &str = "user prompt submit";
const COUNCIL_SESSION_TASK: &str = "council lifecycle";

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> anyhow::Result<()> {
    let envelope = match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            debug_assert!(FAIL_OPEN_LIFECYCLE_CAPTURE);
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

    // B2: detect error patterns and store to hyphae when present
    let error_lines = detect_prompt_error_patterns(&event.prompt);
    if !error_lines.is_empty() && command_exists("hyphae") {
        let project = project_name_for_cwd(Some(&event.cwd));
        let error_content = serde_json::json!({
            "type": "prompt_error_patterns",
            "session_id": event.session_id,
            "cwd": event.cwd,
            "matched_lines": error_lines,
        })
        .to_string();
        let _ = ensure_scoped_hyphae_session(Some(&event.cwd), Some(PROMPT_SESSION_TASK));
        store_in_hyphae(
            "errors/active",
            &error_content,
            Importance::Medium,
            project.as_deref(),
        );
    }

    // B3: extract file references and track them as pending exports
    let file_refs = extract_file_refs(&event.prompt);
    if !file_refs.is_empty() {
        super::post_tool_use::track_prompt_file_refs(&file_refs, &hash);
    }

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

        if let Some(council_content) = council_lifecycle_content(event) {
            if remember_council_capture(&hash, &council_content) {
                let _ = ensure_scoped_hyphae_session(Some(&event.cwd), Some(COUNCIL_SESSION_TASK));
                store_in_hyphae(
                    COUNCIL_TOPIC,
                    &council_content,
                    Importance::High,
                    project.as_deref(),
                );
            }
        }
    }

    // B4: record lightweight outcome after error detection + file extraction
    let outcome = OutcomeEvent::new(
        OutcomeKind::KnowledgeExported,
        format!("prompt captured ({} chars)", event.prompt.len()),
    );
    let _ = record_outcome(&hash, outcome);
}

fn prompt_memory_content(event: &UserPromptSubmitEvent) -> String {
    json!({
        "type": "prompt",
        "normalized_lifecycle_event": {
            "category": "session",
            "status": "captured",
            "host": "claude_code",
            "event_name": "user_prompt_submit",
            "fail_open": FAIL_OPEN_LIFECYCLE_CAPTURE,
        },
        "content": event.prompt,
        "session_id": event.session_id,
        "cwd": event.cwd,
        "transcript_path": event.transcript_path,
    })
    .to_string()
}

fn council_lifecycle_content(event: &UserPromptSubmitEvent) -> Option<String> {
    if !is_council_prompt(&event.prompt) {
        return None;
    }

    let mut normalized = NormalizedLifecycleEvent::from_council_prompt(event);
    annotate_task_linkage(
        &mut normalized,
        Some(&event.cwd),
        scope_identity_for_cwd,
        current_task_id_for_cwd,
    );

    serde_json::to_string(&normalized).ok()
}

fn annotate_task_linkage<SI, TL>(
    normalized: &mut NormalizedLifecycleEvent,
    cwd: Option<&str>,
    scope_identity: SI,
    task_lookup: TL,
) where
    SI: FnOnce(Option<&str>) -> Option<(String, String)>,
    TL: FnOnce(Option<&str>) -> Option<String>,
{
    if let Some((project_root, worktree_id)) = scope_identity(cwd) {
        normalized.project_root = Some(project_root);
        normalized.worktree_id = Some(worktree_id);
    }
    if let Some(task_id) = task_lookup(cwd) {
        normalized
            .metadata
            .insert("task_id".to_string(), json!(task_id));
        normalized
            .metadata
            .insert("task_linked".to_string(), json!(true));
    }
}

/// Scan the prompt for lines that look like error output.
/// Returns up to 5 matching lines, each truncated to 200 chars.
/// Patterns must appear at line start or followed by `:` or `[` to avoid false positives.
fn detect_prompt_error_patterns(prompt: &str) -> Vec<String> {
    const ERROR_PATTERNS: &[&str] = &[
        "error",
        "failed",
        "panicked",
        "FAILED",
        "could not",
        "cannot",
    ];
    const MAX_LINE_LEN: usize = 200;
    const MAX_MATCHES: usize = 5;

    fn matches_pattern(line: &str, pattern: &str) -> bool {
        if line.starts_with(pattern) {
            return true;
        }
        line.contains(&format!("{pattern}:")) || line.contains(&format!("{pattern}["))
    }

    prompt
        .lines()
        .filter(|line| ERROR_PATTERNS.iter().any(|pat| matches_pattern(line, pat)))
        .take(MAX_MATCHES)
        .map(|line| {
            let truncated: String = line.chars().take(MAX_LINE_LEN).collect();
            truncated
        })
        .collect()
}

/// Extract tokens from the prompt that look like file paths.
/// A token qualifies when it contains `/`, has a file extension,
/// and is between 3 and 512 chars long. Returns at most 10 unique results.
/// URLs are filtered out.
pub(crate) fn extract_file_refs(prompt: &str) -> Vec<String> {
    const MIN_LEN: usize = 3;
    const MAX_LEN: usize = 512;
    const MAX_REFS: usize = 10;

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut results: Vec<String> = Vec::new();

    for token in prompt.split_whitespace() {
        if results.len() >= MAX_REFS {
            break;
        }
        let clean = token.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-'
        });
        if clean.len() < MIN_LEN || clean.len() > MAX_LEN {
            continue;
        }
        if clean.starts_with("http://") || clean.starts_with("https://") {
            continue;
        }
        if !clean.contains('/') {
            continue;
        }
        // Must have a file extension (a dot after the last slash)
        let after_last_slash = clean.rsplit('/').next().unwrap_or("");
        if !after_last_slash.contains('.') {
            continue;
        }
        if seen.insert(clean.to_string()) {
            results.push(clean.to_string());
        }
    }

    results
}

fn prompt_capture_state_path(hash: &str) -> PathBuf {
    temp_state_path("prompt-captures", hash, "json")
}

fn council_capture_state_path(hash: &str) -> PathBuf {
    temp_state_path("council-captures", hash, "json")
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

fn remember_council_capture(hash: &str, content: &str) -> bool {
    update_json_file::<Vec<String>, _, _>(&council_capture_state_path(hash), |captures| {
        if captures.iter().any(|existing| existing == content) {
            return false;
        }

        captures.push(content.to_string());
        if captures.len() > MAX_RECORDED_COUNCIL_EVENTS {
            let overflow = captures.len().saturating_sub(MAX_RECORDED_COUNCIL_EVENTS);
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
        assert!(content.contains(r#""normalized_lifecycle_event":{"category":"session""#));
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

    #[test]
    fn builds_council_lifecycle_content_for_council_prompts() {
        let event = UserPromptSubmitEvent {
            session_id: "abc123".to_string(),
            cwd: "/tmp/demo".to_string(),
            prompt: "/council review the unresolved failures".to_string(),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
        };

        let content = council_lifecycle_content(&event).expect("council content");
        assert!(content.contains(r#""category":"council""#));
        assert!(content.contains(r#""event_name":"user_prompt_submit""#));
        assert!(content.contains(r#""prompt_excerpt":"/council review the unresolved failures""#));
    }

    #[test]
    fn council_lifecycle_content_includes_task_identity_when_available() {
        let event = UserPromptSubmitEvent {
            session_id: "abc123".to_string(),
            cwd: "/tmp/demo".to_string(),
            prompt: "/council review the unresolved failures".to_string(),
            transcript_path: Some("/tmp/transcript.jsonl".to_string()),
        };

        let mut normalized = NormalizedLifecycleEvent::from_council_prompt(&event);
        annotate_task_linkage(
            &mut normalized,
            Some(&event.cwd),
            |_| Some(("/tmp/demo".to_string(), "git:demo".to_string())),
            |_| Some("task-123".to_string()),
        );

        let parsed = serde_json::to_value(&normalized).expect("valid json");
        assert_eq!(parsed["project_root"].as_str(), Some("/tmp/demo"));
        assert_eq!(parsed["worktree_id"].as_str(), Some("git:demo"));
        assert_eq!(parsed["metadata"]["task_id"].as_str(), Some("task-123"));
        assert_eq!(parsed["metadata"]["task_linked"].as_bool(), Some(true));
    }

    #[test]
    fn ignores_non_council_prompts_for_council_capture() {
        let event = UserPromptSubmitEvent {
            session_id: "abc123".to_string(),
            cwd: "/tmp/demo".to_string(),
            prompt: "summarize current work".to_string(),
            transcript_path: None,
        };

        assert!(council_lifecycle_content(&event).is_none());
    }

    #[test]
    fn detect_prompt_error_patterns_returns_matching_lines() {
        let prompt = "All good\nerror: compilation failed\nFAILED: cargo test\neverything fine";
        let patterns = detect_prompt_error_patterns(prompt);
        assert_eq!(patterns.len(), 2);
        assert!(patterns[0].contains("error:"));
        assert!(patterns[1].contains("FAILED:"));
    }

    #[test]
    fn detect_prompt_error_patterns_returns_empty_when_no_matches() {
        let prompt = "implement the feature\nadd tests\nship it";
        let patterns = detect_prompt_error_patterns(prompt);
        assert!(patterns.is_empty());
    }

    #[test]
    fn detect_prompt_error_patterns_truncates_long_lines() {
        let long_line = "error: ".to_string() + &"x".repeat(300);
        let patterns = detect_prompt_error_patterns(&long_line);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].chars().count(), 200);
    }

    #[test]
    fn detect_prompt_error_patterns_limits_to_five_matches() {
        let many_errors = (0..10)
            .map(|i| format!("error: failure {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let patterns = detect_prompt_error_patterns(&many_errors);
        assert_eq!(patterns.len(), 5);
    }

    #[test]
    fn extract_file_refs_finds_path_like_tokens() {
        let prompt = "please read src/main.rs and also check tests/integration.rs";
        let refs = extract_file_refs(prompt);
        assert!(refs.contains(&"src/main.rs".to_string()));
        assert!(refs.contains(&"tests/integration.rs".to_string()));
    }

    #[test]
    fn extract_file_refs_ignores_tokens_without_extension() {
        let prompt = "look at src/lib and also /tmp/dir/";
        let refs = extract_file_refs(prompt);
        assert!(!refs.contains(&"src/lib".to_string()));
        assert!(!refs.contains(&"/tmp/dir/".to_string()));
    }

    #[test]
    fn extract_file_refs_returns_unique_results() {
        let prompt = "check src/main.rs and src/main.rs again";
        let refs = extract_file_refs(prompt);
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn extract_file_refs_limits_to_ten_results() {
        let paths: Vec<String> = (0..15).map(|i| format!("src/file{i}.rs")).collect();
        let prompt = paths.join(" ");
        let refs = extract_file_refs(&prompt);
        assert!(refs.len() <= 10);
    }

    #[test]
    fn extract_file_refs_ignores_urls() {
        let prompt = "see https://docs.rs/serde/latest/serde.html for docs";
        let refs = extract_file_refs(prompt);
        assert!(refs.is_empty());
    }
}

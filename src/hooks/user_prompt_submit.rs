use serde_json::json;
use std::collections::{BTreeSet, HashSet};
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
    project_name_for_cwd, resolved_command, scope_hash, scope_identity_for_cwd, store_in_hyphae,
    temp_state_path, update_json_file,
};

const MAX_RECORDED_PROMPTS: usize = 32;
const MAX_RECORDED_COUNCIL_EVENTS: usize = 16;
const PROMPT_TOPIC: &str = "session/prompts";
const COUNCIL_TOPIC: &str = "session/council-lifecycle";
const PROMPT_SESSION_TASK: &str = "user prompt submit";
const COUNCIL_SESSION_TASK: &str = "council lifecycle";
const RECALL_TOPIC: &str = "session/recall";

/// Maximum characters for the hyphae search query extracted from the prompt.
const RECALL_QUERY_MAX_CHARS: usize = 200;
/// Approximate char budget for recalled content (2000 chars ≈ 500 tokens).
const RECALL_CHAR_BUDGET: usize = 2_000;
/// Maximum number of memories to request from hyphae.
const RECALL_LIMIT: usize = 5;

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> anyhow::Result<()> {
    let envelope = match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            const { assert!(FAIL_OPEN_LIFECYCLE_CAPTURE) };
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

    // B5: query hyphae for memories relevant to this prompt and surface them
    inject_recall(event, &hash);

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

/// Return the path used to persist the set of memory IDs already surfaced this session.
fn recall_seen_state_path(hash: &str) -> PathBuf {
    temp_state_path("recall-seen", hash, "json")
}

/// Extract a recall query from the prompt: up to `RECALL_QUERY_MAX_CHARS` characters,
/// trimmed to the last sentence boundary when possible.
fn recall_query(prompt: &str) -> String {
    let excerpt: String = prompt.chars().take(RECALL_QUERY_MAX_CHARS).collect();
    // Try to trim at a sentence-ending punctuation to keep the query coherent.
    let boundary_chars = ['.', '!', '?', '\n'];
    if let Some(pos) = excerpt.rfind(|c| boundary_chars.contains(&c)) {
        let candidate = excerpt[..=pos].trim().to_string();
        if !candidate.is_empty() {
            return candidate;
        }
    }
    excerpt.trim().to_string()
}

/// A memory entry parsed from `hyphae search --json` output.
#[derive(Debug)]
struct RecalledMemory {
    id: String,
    content: String,
}

/// Call `hyphae search --json` and parse the returned memories.
/// Returns an empty list if hyphae is unavailable or the call fails.
fn query_hyphae_recall(query: &str, project: Option<&str>) -> Vec<RecalledMemory> {
    let Some(mut cmd) = resolved_command("hyphae") else {
        return Vec::new();
    };

    cmd.args(["search", "--query", query])
        .args(["--limit", &RECALL_LIMIT.to_string()])
        .arg("--json");

    if let Some(proj) = project {
        cmd.args(["-P", proj]);
    }

    let Ok(output) = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };

    if !output.status.success() {
        return Vec::new();
    }

    let Ok(text) = std::str::from_utf8(&output.stdout) else {
        return Vec::new();
    };

    parse_recall_json(text)
}

/// Parse the JSON emitted by `hyphae search --json` into a list of recalled memories.
fn parse_recall_json(text: &str) -> Vec<RecalledMemory> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };

    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    results
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.to_string();
            // Prefer `summary` for a compact view; fall back to the raw excerpt.
            let content = entry
                .get("summary")
                .and_then(|v| v.as_str())
                .or_else(|| entry.get("raw_excerpt").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();
            if content.is_empty() {
                return None;
            }
            Some(RecalledMemory { id, content })
        })
        .collect()
}

/// Load the set of already-seen memory IDs from the scoped state file.
fn load_recall_seen(hash: &str) -> HashSet<String> {
    let path = recall_seen_state_path(hash);
    crate::utils::load_json_file::<Vec<String>>(&path)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// Persist new memory IDs to the seen-set state file, merging with existing entries.
fn save_recall_seen(hash: &str, new_ids: &[String]) {
    if new_ids.is_empty() {
        return;
    }
    let path = recall_seen_state_path(hash);
    let _ = update_json_file::<Vec<String>, _, _>(&path, |seen| {
        let existing: HashSet<_> = seen.iter().cloned().collect();
        for id in new_ids {
            if existing.contains(id) {
                continue;
            }
            seen.push(id.clone());
        }
    });
}

/// Apply the character budget: accumulate memories until `RECALL_CHAR_BUDGET` is reached.
fn budget_memories(memories: Vec<RecalledMemory>) -> Vec<RecalledMemory> {
    let mut total = 0usize;
    let mut out = Vec::new();
    for m in memories {
        let len = m.content.len();
        if total + len > RECALL_CHAR_BUDGET {
            break;
        }
        total += len;
        out.push(m);
    }
    out
}

/// Query hyphae for memories relevant to the incoming prompt, deduplicate within the
/// session window, apply the token budget, and surface the results to stderr.
///
/// Gracefully degrades: if hyphae is unavailable or any step fails the handler
/// continues normally without recall.
fn inject_recall(event: &UserPromptSubmitEvent, hash: &str) {
    if !command_exists("hyphae") {
        return;
    }

    let query = recall_query(&event.prompt);
    if query.is_empty() {
        return;
    }

    let project = project_name_for_cwd(Some(&event.cwd));
    let memories = query_hyphae_recall(&query, project.as_deref());
    if memories.is_empty() {
        return;
    }

    // Deduplicate against memories already surfaced this session.
    let seen = load_recall_seen(hash);
    let fresh: Vec<RecalledMemory> = memories
        .into_iter()
        .filter(|m| !seen.contains(&m.id))
        .collect();

    if fresh.is_empty() {
        return;
    }

    // Apply the token budget.
    let budgeted = budget_memories(fresh);
    if budgeted.is_empty() {
        return;
    }

    let injected_ids: Vec<String> = budgeted.iter().map(|m| m.id.clone()).collect();
    let n = budgeted.len();

    // Surface the recall block to stderr so the agent can see it.
    eprintln!(
        "[cortina-recall] {} memories injected for session {}",
        n, event.session_id
    );
    for memory in &budgeted {
        eprintln!("[cortina-recall] id={} content={}", memory.id, memory.content);
    }

    // Persist the seen IDs to avoid re-surfacing them in future turns.
    save_recall_seen(hash, &injected_ids);

    // Store the recall event in hyphae using the fire-and-forget pattern.
    let recall_payload = json!({
        "type": "recall_injection",
        "session_id": event.session_id,
        "cwd": event.cwd,
        "query_excerpt": query,
        "injected_count": n,
        "injected_ids": injected_ids,
    })
    .to_string();
    store_in_hyphae(
        RECALL_TOPIC,
        &recall_payload,
        Importance::Medium,
        project.as_deref(),
    );
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

    // ---- recall injection helpers ----

    #[test]
    fn recall_query_trims_to_sentence_boundary() {
        // Prompt longer than RECALL_QUERY_MAX_CHARS with a sentence boundary before
        // the cut point — the query must trim at the boundary, not mid-string.
        let mut prompt = "First sentence ends here. ".to_string();
        prompt.push_str(&"x".repeat(300)); // total > RECALL_QUERY_MAX_CHARS
        let q = recall_query(&prompt);
        assert!(
            q.ends_with('.'),
            "expected trim at sentence boundary, got: {q:?}"
        );
        assert!(q.chars().count() <= RECALL_QUERY_MAX_CHARS);
        assert!(q.contains("First sentence ends here"));
    }

    #[test]
    fn recall_query_truncates_long_prompts() {
        let long = "a".repeat(500);
        let q = recall_query(&long);
        assert!(q.chars().count() <= RECALL_QUERY_MAX_CHARS);
    }

    #[test]
    fn recall_query_returns_empty_for_empty_prompt() {
        let q = recall_query("   ");
        assert!(q.is_empty());
    }

    #[test]
    fn parse_recall_json_extracts_memories() {
        let json = r#"{
            "schema_version": "1.0",
            "results": [
                {"id": "AAA", "summary": "first memory", "raw_excerpt": "raw one"},
                {"id": "BBB", "summary": "second memory", "raw_excerpt": "raw two"}
            ]
        }"#;
        let memories = parse_recall_json(json);
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0].id, "AAA");
        assert_eq!(memories[0].content, "first memory");
        assert_eq!(memories[1].id, "BBB");
    }

    #[test]
    fn parse_recall_json_falls_back_to_raw_excerpt_when_no_summary() {
        let json = r#"{"results": [{"id": "CCC", "raw_excerpt": "the excerpt"}]}"#;
        let memories = parse_recall_json(json);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "the excerpt");
    }

    #[test]
    fn parse_recall_json_skips_entries_with_empty_content() {
        let json = r#"{"results": [{"id": "DDD", "summary": ""}, {"id": "EEE", "summary": "ok"}]}"#;
        let memories = parse_recall_json(json);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].id, "EEE");
    }

    #[test]
    fn parse_recall_json_returns_empty_for_invalid_json() {
        let memories = parse_recall_json("not json at all");
        assert!(memories.is_empty());
    }

    #[test]
    fn budget_memories_respects_char_limit() {
        let memories: Vec<RecalledMemory> = (0..10)
            .map(|i| RecalledMemory {
                id: format!("ID{i}"),
                content: "x".repeat(600),
            })
            .collect();
        let budgeted = budget_memories(memories);
        let total_chars: usize = budgeted.iter().map(|m| m.content.len()).sum();
        assert!(total_chars <= RECALL_CHAR_BUDGET);
    }

    #[test]
    fn budget_memories_includes_first_entry_even_at_limit() {
        let memories = vec![RecalledMemory {
            id: "ID0".to_string(),
            content: "x".repeat(100),
        }];
        let budgeted = budget_memories(memories);
        assert_eq!(budgeted.len(), 1);
    }

    #[test]
    fn budget_memories_drops_all_when_first_exceeds_budget() {
        // When the first memory alone exceeds RECALL_CHAR_BUDGET, budget_memories
        // returns empty. This documents the silent-drop behavior so callers are aware.
        let memories = vec![RecalledMemory {
            id: "big".to_string(),
            content: "x".repeat(RECALL_CHAR_BUDGET + 1),
        }];
        let budgeted = budget_memories(memories);
        assert!(
            budgeted.is_empty(),
            "oversized first memory should be excluded by budget"
        );
    }

    #[test]
    fn save_and_load_recall_seen_persists_ids() {
        let hash = scope_hash(Some("/tmp/recall-test"));
        let path = recall_seen_state_path(&hash);
        let _ = remove_file_with_lock(&path);

        save_recall_seen(&hash, &["AAA".to_string(), "BBB".to_string()]);
        let seen = load_recall_seen(&hash);
        assert!(seen.contains("AAA"));
        assert!(seen.contains("BBB"));

        // Adding again should not duplicate.
        save_recall_seen(&hash, &["AAA".to_string(), "CCC".to_string()]);
        let seen2 = load_recall_seen(&hash);
        assert_eq!(seen2.len(), 3);
        assert!(seen2.contains("CCC"));

        let _ = remove_file_with_lock(&path);
    }
}

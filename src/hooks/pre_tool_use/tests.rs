use std::fmt::Write as _;
use std::fs;

use crate::tool_usage::{ToolSource, clear_tool_calls, record_tool_call};
use crate::utils::remove_file_with_lock;

use super::*;

#[test]
fn read_advisory_skips_non_code_files() {
    assert!(read_advisory_for_path("README.md", None, 100).is_none());
    assert!(read_advisory_for_path(".env", None, 100).is_none());
}

#[test]
fn read_advisory_skips_small_code_files() {
    let temp_dir = std::env::temp_dir().join("cortina-read-suggestion-small");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("small.rs");
    fs::write(&file_path, "fn main() {}\n").unwrap();

    assert!(read_advisory_for_path(file_path.to_str().unwrap(), None, 100).is_none());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn read_advisory_triggers_for_large_code_files() {
    let temp_dir = std::env::temp_dir().join("cortina-read-suggestion-large");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("large.rs");
    let content = (0..101).fold(String::new(), |mut content, i| {
        writeln!(&mut content, "fn line_{i}() {{}}").unwrap();
        content
    });
    fs::write(&file_path, content).unwrap();

    let advisory = read_advisory_for_path(file_path.to_str().unwrap(), None, 100)
        .expect("large code file should trigger an advisory");
    assert_eq!(advisory.message, READ_ADVISORY_MESSAGE);
    assert_eq!(advisory.rate_limit_key, "read:rs");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn read_advisory_resolves_relative_paths_from_event_cwd() {
    let temp_dir = std::env::temp_dir().join("cortina-read-suggestion-relative");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("src")).unwrap();
    let file_path = temp_dir.join("src/large.rs");
    let content = (0..101).fold(String::new(), |mut content, i| {
        writeln!(&mut content, "fn line_{i}() {{}}").unwrap();
        content
    });
    fs::write(&file_path, content).unwrap();

    let advisory = read_advisory_for_path("src/large.rs", temp_dir.to_str(), 100)
        .expect("relative path should resolve using event cwd");
    assert_eq!(advisory.rate_limit_key, "read:rs");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn grep_advisory_matches_symbol_like_patterns() {
    assert_eq!(symbol_like_grep_kind("AuthService"), Some("type"));
    assert_eq!(symbol_like_grep_kind("parse_command"), Some("symbol"));
    assert_eq!(symbol_like_grep_kind("handleRequest"), Some("symbol"));
    assert_eq!(symbol_like_grep_kind("parse_command("), Some("call"));

    let advisory = grep_advisory_for_pattern("AuthService").expect("symbol search should advise");
    assert_eq!(advisory.message, GREP_ADVISORY_MESSAGE);
    assert_eq!(advisory.rate_limit_key, "grep:type");
}

#[test]
fn grep_advisory_skips_regex_patterns() {
    assert_eq!(symbol_like_grep_kind("foo.*bar"), None);
    assert_eq!(symbol_like_grep_kind("^fn "), None);
    assert_eq!(symbol_like_grep_kind("id"), None);
    assert!(grep_advisory_for_pattern("TODO|FIXME").is_none());
}

#[test]
fn tool_suggestion_requires_rhizome_availability() {
    let envelope = ClaudeCodeHookEnvelope::parse(
        r#"{
            "tool_name": "Grep",
            "tool_input": {"pattern": "AuthService"},
            "cwd": "/tmp/cortina"
        }"#,
    )
    .expect("valid envelope");
    let policy = CapturePolicy::from_reader(|_| None);

    assert!(tool_suggestion_message_with_availability(&policy, &envelope, false).is_none());
}

#[test]
fn advisory_rate_limiting_emits_once_per_cadence() {
    let temp_dir = std::env::temp_dir().join(format!(
        "cortina-advisory-rate-limit-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let scope = temp_dir.to_str().unwrap();
    let hash = scope_hash(Some(scope));
    let path = temp_state_path(ADVISORY_STATE_NAME, &hash, "json");

    assert!(advisory_allowed(Some(scope), "read:rs", 5));
    for _ in 0..4 {
        assert!(!advisory_allowed(Some(scope), "read:rs", 5));
    }
    assert!(advisory_allowed(Some(scope), "read:rs", 5));
    assert!(advisory_allowed(Some(scope), "grep:type", 5));

    let _ = remove_file_with_lock(&path);
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn tool_suggestion_respects_rate_limit_per_scope_and_pattern_type() {
    let temp_dir =
        std::env::temp_dir().join(format!("cortina-advisory-scope-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let cwd = temp_dir.to_string_lossy().into_owned();
    let hash = scope_hash(Some(&cwd));
    let path = temp_state_path(ADVISORY_STATE_NAME, &hash, "json");
    let envelope = ClaudeCodeHookEnvelope::parse(
        &serde_json::json!({
            "tool_name": "Grep",
            "tool_input": {"pattern": "AuthService"},
            "cwd": cwd,
        })
        .to_string(),
    )
    .expect("valid envelope");
    let policy = CapturePolicy::from_reader(|name| match name {
        "CORTINA_RHIZOME_SUGGEST_EVERY" => Some("2".to_string()),
        _ => None,
    });

    assert!(tool_suggestion_message_with_availability(&policy, &envelope, true).is_some());
    assert!(tool_suggestion_message_with_availability(&policy, &envelope, true).is_none());
    assert!(tool_suggestion_message_with_availability(&policy, &envelope, true).is_some());

    let _ = remove_file_with_lock(&path);
    let _ = fs::remove_dir_all(&temp_dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// pre_write advisory tests
// ─────────────────────────────────────────────────────────────────────────────

fn pre_write_test_cwd(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("cortina-pre-write-{}-{}", std::process::id(), label))
}

#[test]
fn pre_write_advisory_emitted_for_rs_file_without_rhizome_calls() {
    let cwd = pre_write_test_cwd("no-rhizome");
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir_all(&cwd).unwrap();
    let cwd_str = cwd.to_str().unwrap();

    // Seed session with a non-rhizome call so session is not empty
    let hash = scope_hash(Some(cwd_str));
    record_tool_call("Read", ToolSource::Other, &hash);

    let advisory = write_advisory(Some(cwd_str), "Write", Some("src/lib.rs"));
    assert!(
        advisory.is_some(),
        "expected advisory when no rhizome tool called"
    );
    let advisory = advisory.unwrap();
    assert!(
        advisory.message.contains("[cortina]"),
        "message should start with cortina tag"
    );
    assert!(
        advisory.rate_limit_key.starts_with("pre_write:Write:"),
        "rate limit key should include operation"
    );

    clear_tool_calls(&hash);
    let _ = fs::remove_dir_all(&cwd);
}

#[test]
fn pre_write_advisory_suppressed_when_rhizome_already_called() {
    let cwd = pre_write_test_cwd("with-rhizome");
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir_all(&cwd).unwrap();
    let cwd_str = cwd.to_str().unwrap();

    let hash = scope_hash(Some(cwd_str));
    record_tool_call("mcp__rhizome__get_symbols", ToolSource::Rhizome, &hash);

    let advisory = write_advisory(Some(cwd_str), "Edit", Some("src/main.rs"));
    assert!(
        advisory.is_none(),
        "no advisory expected when rhizome tool was called"
    );

    clear_tool_calls(&hash);
    let _ = fs::remove_dir_all(&cwd);
}

#[test]
fn pre_write_advisory_not_emitted_for_markdown_files() {
    let cwd = pre_write_test_cwd("md-file");
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir_all(&cwd).unwrap();
    let cwd_str = cwd.to_str().unwrap();

    let hash = scope_hash(Some(cwd_str));
    record_tool_call("Bash", ToolSource::Other, &hash);

    let advisory = write_advisory(Some(cwd_str), "Write", Some("README.md"));
    assert!(
        advisory.is_none(),
        "no advisory expected for markdown files"
    );

    clear_tool_calls(&hash);
    let _ = fs::remove_dir_all(&cwd);
}

#[test]
fn pre_write_advisory_not_emitted_when_session_has_no_tool_calls() {
    let cwd = pre_write_test_cwd("empty-session");
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir_all(&cwd).unwrap();
    let cwd_str = cwd.to_str().unwrap();

    // Do NOT record any tool calls — fresh session
    let advisory = write_advisory(Some(cwd_str), "Write", Some("src/lib.rs"));
    assert!(
        advisory.is_none(),
        "no advisory expected when session just started"
    );

    let _ = fs::remove_dir_all(&cwd);
}

#[test]
fn write_advisory_emitted_through_full_suggestion_path_for_rs_file() {
    let temp_dir = std::env::temp_dir().join(format!(
        "cortina-advisory-write-full-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let cwd = temp_dir.to_string_lossy().into_owned();

    // Seed a non-rhizome tool call so the session is not empty
    let hash = scope_hash(Some(&cwd));
    record_tool_call("Read", ToolSource::Other, &hash);

    let envelope = ClaudeCodeHookEnvelope::parse(
        &serde_json::json!({
            "tool_name": "Write",
            "tool_input": {"file_path": "src/lib.rs", "content": "fn main() {}"},
            "cwd": cwd,
        })
        .to_string(),
    )
    .expect("valid envelope");

    let policy = CapturePolicy::from_reader(|_| None);

    let suggestion = tool_suggestion_message_with_availability(&policy, &envelope, true);
    assert!(
        suggestion.is_some(),
        "expected advisory when writing a .rs file without prior rhizome calls"
    );
    let message = suggestion.unwrap();
    assert!(
        message.contains("[cortina]"),
        "advisory message should contain cortina tag"
    );

    clear_tool_calls(&hash);
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn span_context_includes_session_id_when_present() {
    let envelope = ClaudeCodeHookEnvelope::parse(
        r#"{
            "session_id": "abc123",
            "cwd": "/tmp/demo",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"}
        }"#,
    )
    .expect("valid envelope");

    let context = span_context(&envelope);
    assert_eq!(context.session_id.as_deref(), Some("abc123"));
    assert_eq!(context.workspace_root.as_deref(), Some("/tmp/demo"));
}

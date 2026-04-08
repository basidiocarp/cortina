use std::fs;

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
    let content = (0..101)
        .map(|i| format!("fn line_{i}() {{}}\n"))
        .collect::<String>();
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
    let content = (0..101)
        .map(|i| format!("fn line_{i}() {{}}\n"))
        .collect::<String>();
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
    let cwd = temp_dir.to_str().unwrap();
    let hash = scope_hash(Some(cwd));
    let path = temp_state_path(ADVISORY_STATE_NAME, &hash, "json");
    let envelope = ClaudeCodeHookEnvelope::parse(&format!(
        r#"{{
            "tool_name": "Grep",
            "tool_input": {{"pattern": "AuthService"}},
            "cwd": "{cwd}"
        }}"#
    ))
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

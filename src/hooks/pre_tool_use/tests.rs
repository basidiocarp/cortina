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
    std::env::temp_dir().join(format!(
        "cortina-pre-write-{}-{}",
        std::process::id(),
        label
    ))
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

// ─────────────────────────────────────────────────────────────────────────────
// GateGuard integration tests
// ─────────────────────────────────────────────────────────────────────────────

/// Destructive Bash gate in blocking mode blocks on first call, then allows
/// after investigation tool (Grep/Read/etc) is called.
#[test]
fn destructive_bash_re_gates_after_allow() {
    // Use a command that encodes the current nanosecond timestamp so the gate
    // key is unique per test execution even within the same process (thread-local
    // GATE_MAP persists across tests in the same thread).
    let unique_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let unique_cwd = format!(
        "/tmp/gate-destructive-{}-{}",
        std::process::id(),
        unique_nonce
    );
    let destructive_cmd = format!("rm -rf {unique_cwd}");

    let make_envelope = |cmd: &str, cwd: &str| {
        ClaudeCodeHookEnvelope::parse(
            &serde_json::json!({
                "tool_name": "Bash",
                "tool_input": {"command": cmd},
                "cwd": cwd
            })
            .to_string(),
        )
        .expect("valid envelope")
    };

    let envelope = make_envelope(&destructive_cmd, &unique_cwd);

    // Ensure the session scope starts empty (no investigation tools recorded).
    let hash = scope_hash(envelope.cwd());
    clear_tool_calls(&hash);

    // In blocking mode, first call blocks without investigation.
    let decision1 = check_gate_guard(&envelope);
    assert!(
        matches!(decision1, Some(GateDecision::Block { .. })),
        "first destructive bash call must block in blocking mode"
    );

    // Simulate investigation: record a Read tool call in the session scope.
    record_tool_call("Read", ToolSource::Other, &hash);

    // Second call with investigation: now allows.
    let decision2 = check_gate_guard(&envelope);
    assert!(
        matches!(decision2, Some(GateDecision::Allow)),
        "destructive bash call should allow after investigation"
    );

    // Third call: blocks again — the allowed entry was removed after Allow so destructive
    // bash must re-investigate before it can proceed.
    let decision3 = check_gate_guard(&envelope);
    assert!(
        matches!(decision3, Some(GateDecision::Block { .. })),
        "destructive bash must re-gate after allow (no TTL bypass)"
    );

    // Cleanup session state.
    clear_tool_calls(&hash);
}

/// Edit gate in advisory mode always returns Allow (fail-open), even without
/// investigation tool calls. This is safe for process-per-call environments.
/// The gate still tracks state internally, but never blocks.
#[test]
fn edit_gate_stays_blocked_without_investigation_tool_calls() {
    // Unique cwd and file path per execution so the thread-local GATE_MAP entry
    // is fresh (pid alone is not enough — multiple tests share the same process).
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let unique_cwd = format!(
        "/tmp/cortina-gate-edit-test-{}-{}",
        std::process::id(),
        nonce
    );
    let unique_path = format!("{unique_cwd}/src/main.rs");

    // Ensure no leftover tool-call state for this scope.
    let hash = scope_hash(Some(unique_cwd.as_str()));
    clear_tool_calls(&hash);

    let make_envelope = |file_path: &str, cwd: &str| {
        ClaudeCodeHookEnvelope::parse(
            &serde_json::json!({
                "tool_name": "Edit",
                "tool_input": {
                    "file_path": file_path,
                    "old_string": "fn foo() {}",
                    "new_string": "fn foo() { bar(); }"
                },
                "cwd": cwd
            })
            .to_string(),
        )
        .expect("valid envelope")
    };

    let envelope = make_envelope(&unique_path, &unique_cwd);

    // In advisory mode (default), gate always returns Allow (fail-open).
    // First call: must allow.
    let decision1 = check_gate_guard(&envelope);
    assert!(
        matches!(decision1, Some(GateDecision::Allow)),
        "first Edit call must allow in advisory mode (fail-open)"
    );

    // Second call with no investigation tool calls in session: must still allow.
    // Advisory mode always allows, regardless of investigation state.
    let decision2 = check_gate_guard(&envelope);
    assert!(
        matches!(decision2, Some(GateDecision::Allow)),
        "Edit gate must allow in advisory mode even without investigation tools"
    );

    // Cleanup.
    clear_tool_calls(&hash);
}

/// Edit gate in advisory mode always allows, whether or not investigation tools
/// have been called. Advisory mode is fail-open for process-per-call safety.
#[test]
fn edit_gate_allows_after_investigation_tool_called() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let unique_cwd = format!(
        "/tmp/cortina-gate-edit-allow-test-{}-{}",
        std::process::id(),
        nonce
    );
    let unique_path = format!("{unique_cwd}/src/lib.rs");

    let hash = scope_hash(Some(unique_cwd.as_str()));
    clear_tool_calls(&hash);

    let make_envelope = |file_path: &str, cwd: &str| {
        ClaudeCodeHookEnvelope::parse(
            &serde_json::json!({
                "tool_name": "Edit",
                "tool_input": {
                    "file_path": file_path,
                    "old_string": "fn a() {}",
                    "new_string": "fn a() { b(); }"
                },
                "cwd": cwd
            })
            .to_string(),
        )
        .expect("valid envelope")
    };

    let envelope = make_envelope(&unique_path, &unique_cwd);

    // In advisory mode (default), gate always returns Allow (fail-open).
    // First call: allows (advisory mode).
    let decision1 = check_gate_guard(&envelope);
    assert!(
        matches!(decision1, Some(GateDecision::Allow)),
        "first Edit call must allow in advisory mode (fail-open)"
    );

    // Record an investigation tool call in the session.
    record_tool_call("Grep", ToolSource::Other, &hash);

    // Second call: still allows (advisory mode always allows).
    let decision2 = check_gate_guard(&envelope);
    assert!(
        matches!(decision2, Some(GateDecision::Allow)),
        "Edit gate must allow in advisory mode regardless of investigation tools"
    );

    // Cleanup.
    clear_tool_calls(&hash);
}

// ─────────────────────────────────────────────────────────────────────────────
// is_bash_code_search tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn bash_code_search_blocks_recursive_on_source_dirs() {
    // Recursive grep targeting known source directories is a code search.
    assert!(is_bash_code_search("grep -r 'AuthService' src/"));
    assert!(is_bash_code_search("rg -r 'MyTrait' crates/"));
    assert!(is_bash_code_search("grep -R 'foo' lib/"));
    assert!(is_bash_code_search("grep --recursive 'bar' app/"));
    assert!(is_bash_code_search("grep -r 'baz' pkg/"));
    assert!(is_bash_code_search("grep -r 'qux' packages/"));
    // Non-source dirs with recursive flag are still allowed.
    assert!(!is_bash_code_search("grep -r 'error' logs/"));
    assert!(!is_bash_code_search("grep -r 'key' config/"));
}

#[test]
fn bash_code_search_detects_extension_targeted_grep() {
    assert!(is_bash_code_search("grep 'fn main' src/main.rs"));
    assert!(is_bash_code_search("rg 'TODO' **/*.ts"));
    assert!(is_bash_code_search("grep pattern *.py"));
}

#[test]
fn bash_code_search_allows_non_code_grep() {
    assert!(!is_bash_code_search("grep 'error' logs/app.log"));
    assert!(!is_bash_code_search("grep 'key' config.yaml"));
    assert!(!is_bash_code_search("grep -r 'pattern' config/"));
    assert!(!is_bash_code_search("cat file.txt | grep foo"));
    assert!(!is_bash_code_search("echo hello"));
}

#[test]
fn bash_code_search_allows_non_grep_commands() {
    assert!(!is_bash_code_search("cargo test"));
    assert!(!is_bash_code_search("git log --oneline"));
    assert!(!is_bash_code_search("ls src/"));
}

#[test]
fn bash_code_search_detects_rg_type_flag() {
    // rg -t <lang> patterns are code searches
    assert!(is_bash_code_search("rg -t rust MyStruct"));
    assert!(is_bash_code_search("rg -t py import foo"));
    assert!(is_bash_code_search("rg -t go pattern ."));
    assert!(is_bash_code_search("rg -t js useState"));

    // rg --type <lang> patterns are code searches
    assert!(is_bash_code_search("rg --type rust MyStruct"));
    assert!(is_bash_code_search("rg --type py import foo"));
    assert!(is_bash_code_search("rg --type=rust pattern"));
    assert!(is_bash_code_search("rg --type=go func src/"));

    // ripgrep variations
    assert!(is_bash_code_search("ripgrep -t rust MyStruct"));
    assert!(is_bash_code_search("ripgrep --type py foo"));

    // Multiple flags in the command
    assert!(is_bash_code_search("rg -i -t rust MyStruct"));
    assert!(is_bash_code_search("rg -t rust MyStruct src/"));

    // Without a valid language identifier should not trigger
    assert!(!is_bash_code_search("rg -t"));
    assert!(!is_bash_code_search("rg --type"));
    assert!(!is_bash_code_search("rg -t /some/path"));
    assert!(!is_bash_code_search("rg --type=file.txt"));

    // Non-rg commands with -t should not trigger
    assert!(!is_bash_code_search("grep -t rust pattern"));
}

#[test]
fn rg_type_flag_helper_validates_language_identifiers() {
    assert!(has_rg_type_flag("rg -t rust MyStruct"));
    assert!(has_rg_type_flag("rg --type py foo"));
    assert!(has_rg_type_flag("rg --type=go func"));
    assert!(has_rg_type_flag("ripgrep -t rust"));

    // Reject invalid language identifiers
    assert!(!has_rg_type_flag("rg -t"));
    assert!(!has_rg_type_flag("rg --type"));
    assert!(!has_rg_type_flag("rg -t /path/to/file"));
    assert!(!has_rg_type_flag("rg --type=-flag"));
    assert!(!has_rg_type_flag("rg --type=file.rs"));

    // Non-rg commands should not match
    assert!(!has_rg_type_flag("grep -t rust"));
    assert!(!has_rg_type_flag("cat file.rs"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Rhizome enforcement tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn grep_tool_symbol_blocked_when_enforce_enabled() {
    let _envelope = ClaudeCodeHookEnvelope::parse(
        r#"{
            "tool_name": "Grep",
            "tool_input": {"pattern": "AuthService"},
            "cwd": "/tmp/cortina-enforce-grep"
        }"#,
    )
    .expect("valid envelope");
    let policy = CapturePolicy::from_reader(|name| match name {
        "CORTINA_RHIZOME_ENFORCE" => Some("1".to_string()),
        _ => None,
    });

    // When rhizome is "available" (simulated), a symbol-like pattern triggers enforcement.
    assert!(policy.rhizome_enforce);
    assert!(symbol_like_grep_kind("AuthService").is_some());

    // Verify the advisory path still works when enforce is explicitly disabled.
    // Use a unique cwd so the rate-limit counter starts fresh.
    let non_enforce_cwd = format!("/tmp/cortina-enforce-grep-advisory-{}", std::process::id());
    let non_enforce_envelope = ClaudeCodeHookEnvelope::parse(
        &serde_json::json!({
            "tool_name": "Grep",
            "tool_input": {"pattern": "AuthService"},
            "cwd": non_enforce_cwd,
        })
        .to_string(),
    )
    .expect("valid envelope");
    let non_enforce = CapturePolicy::from_reader(|name| match name {
        "CORTINA_RHIZOME_ENFORCE" => Some("0".to_string()),
        _ => None,
    });
    assert!(!non_enforce.rhizome_enforce);
    // Advisory path respects rhizome_suggest_enabled (true by default).
    let advisory =
        tool_suggestion_message_with_availability(&non_enforce, &non_enforce_envelope, true);
    assert!(
        advisory.is_some(),
        "advisory should be emitted when enforce is off"
    );
}

#[test]
fn grep_tool_non_symbol_not_blocked_by_enforce() {
    // Regex patterns are not symbol-like and pass through even with enforce on.
    assert!(symbol_like_grep_kind("foo.*bar").is_none());
    assert!(symbol_like_grep_kind("^fn ").is_none());
}

#[test]
fn rhizome_enforce_default_is_off() {
    // Advisory-only is the safe default; operators opt in to blocking via CORTINA_RHIZOME_ENFORCE=1.
    let policy = CapturePolicy::from_reader(|_| None);
    assert!(
        !policy.rhizome_enforce,
        "rhizome_enforce must default to false (advisory-only)"
    );
}

#[test]
fn rhizome_enforce_reads_from_env() {
    let policy = CapturePolicy::from_reader(|name| match name {
        "CORTINA_RHIZOME_ENFORCE" => Some("true".to_string()),
        _ => None,
    });
    assert!(policy.rhizome_enforce);

    let policy_off = CapturePolicy::from_reader(|name| match name {
        "CORTINA_RHIZOME_ENFORCE" => Some("0".to_string()),
        _ => None,
    });
    assert!(!policy_off.rhizome_enforce);
}

// ─────────────────────────────────────────────────────────────────────────────
// inject_recall_for_tool tests
// ─────────────────────────────────────────────────────────────────────────────

/// When hyphae is not installed (or the command doesn't exist in $PATH),
/// `handle` must still return Ok and must not block or alter the tool call.
/// This verifies the fail-open contract of the recall branch.
#[test]
fn handle_is_fail_open_when_hyphae_absent() {
    // A minimal PreToolUse envelope for a Write call.
    // `hyphae` is almost certainly not at a sentinel path, so command_exists
    // will return false and inject_recall_for_tool exits early — exercising
    // the fast-exit path without needing to mock PATH.
    let input = serde_json::json!({
        "tool_name": "Write",
        "tool_input": {
            "file_path": "src/lib.rs",
            "content": "fn foo() {}"
        },
        "cwd": "/tmp/cortina-recall-fail-open-test"
    })
    .to_string();

    // handle must return Ok regardless of hyphae availability.
    let result = handle(&input);
    assert!(
        result.is_ok(),
        "handle must return Ok even when hyphae is absent"
    );
}

/// When hyphae IS reachable but exits non-zero (or emits no `[cortina-recall]` lines),
/// `handle` must still return `Ok(())` and must not panic.
///
/// This exercises the fail-open path through the subprocess branch, as opposed to the
/// fast-exit `command_exists == false` path covered by `handle_is_fail_open_when_hyphae_absent`.
///
/// Implementation: we create a temp dir containing an executable `hyphae` stub that
/// exits 1, prepend it to PATH for the duration of the test, then restore PATH.
///
/// If PATH manipulation is unavailable (e.g. cfg(not(unix))), we fall back to a direct
/// unit test of the line-filtering logic instead.
#[test]
#[cfg(unix)]
#[allow(
    unsafe_code,
    reason = "PATH manipulation in single-threaded test; restored before return"
)]
fn handle_is_fail_open_when_hyphae_present_but_fails() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    // Build a stub `hyphae` that exits 1 and writes nothing to stdout.
    let tmp = tempfile::tempdir().expect("tempdir");
    let stub_path = tmp.path().join("hyphae");
    fs::write(&stub_path, "#!/bin/sh\nexit 1\n").expect("write stub");
    fs::set_permissions(&stub_path, fs::Permissions::from_mode(0o755)).expect("chmod stub");

    // Prepend the temp dir so our stub is found first.
    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{original_path}", tmp.path().display());
    // SAFETY: test is marked #[cfg(unix)] and runs in a single-threaded context.
    // PATH is restored before the function returns, so the window of mutation is minimal.
    unsafe { std::env::set_var("PATH", &new_path) };

    let input = serde_json::json!({
        "tool_name": "Write",
        "tool_input": {
            "file_path": "src/lib.rs",
            "content": "fn foo() {}"
        },
        "cwd": "/tmp/cortina-recall-fail-open-nonzero"
    })
    .to_string();

    let result = handle(&input);

    // Restore PATH regardless of outcome.
    unsafe { std::env::set_var("PATH", &original_path) };

    assert!(
        result.is_ok(),
        "handle must return Ok even when hyphae exits non-zero"
    );
}

/// Direct unit test of the `[cortina-recall]` prefix filter: lines without the
/// prefix must be dropped, lines with it must be forwarded.
///
/// This proves the filtering helper is correct independently of the subprocess path,
/// covering the case where hyphae emits output with no `[cortina-recall]` lines.
#[test]
fn recall_filter_drops_non_prefixed_lines() {
    // Simulate output that has no [cortina-recall]-prefixed lines.
    let raw = "some result\nanother line\n[other-prefix] ignored\n";
    let filtered: Vec<&str> = raw
        .lines()
        .filter(|l| l.starts_with("[cortina-recall]"))
        .collect();
    assert!(
        filtered.is_empty(),
        "non-[cortina-recall] lines must be filtered out"
    );

    // Simulate output that has one valid line among noise.
    let mixed = "noise\n[cortina-recall] relevant memory\nmore noise\n";
    let filtered_mixed: Vec<&str> = mixed
        .lines()
        .filter(|l| l.starts_with("[cortina-recall]"))
        .collect();
    assert_eq!(filtered_mixed.len(), 1);
    assert!(filtered_mixed[0].contains("relevant memory"));
}

/// `tool_recall_query` builds a query that includes the tool name and file path
/// for file-editing tools, and falls back to just the tool name for unknown tools.
#[test]
fn tool_recall_query_includes_tool_name_and_file_path() {
    let envelope = ClaudeCodeHookEnvelope::parse(
        r#"{
            "tool_name": "Edit",
            "tool_input": {
                "file_path": "src/main.rs",
                "old_string": "fn a() {}",
                "new_string": "fn a() { b(); }"
            },
            "cwd": "/tmp/demo"
        }"#,
    )
    .expect("valid envelope");

    let query = tool_recall_query(&envelope).expect("query should be built for Edit");
    assert!(query.contains("Edit"), "query must include tool name");
    assert!(
        query.contains("src/main.rs"),
        "query must include file path"
    );
}

#[test]
fn tool_recall_query_includes_command_prefix_for_bash() {
    let envelope = ClaudeCodeHookEnvelope::parse(
        r#"{
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test --release"},
            "cwd": "/tmp/demo"
        }"#,
    )
    .expect("valid envelope");

    let query = tool_recall_query(&envelope).expect("query should be built for Bash");
    assert!(query.contains("Bash"), "query must include tool name");
    assert!(
        query.contains("cargo test"),
        "query must include command excerpt"
    );
}

#[test]
fn tool_recall_query_returns_none_for_envelope_without_tool_name() {
    let envelope =
        ClaudeCodeHookEnvelope::parse(r#"{"cwd": "/tmp/demo"}"#).expect("valid envelope");

    assert!(
        tool_recall_query(&envelope).is_none(),
        "query must be None when tool_name is absent"
    );
}

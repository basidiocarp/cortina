use std::fs;
use std::process::{ExitStatus, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use super::session_scope::{
    clear_session_state, end_hyphae_session_with, ensure_hyphae_session_with_hash,
    session_state_path,
};
use super::state::{lock_path_for, save_json_file};
use super::*;

fn output_with_status(code: i32, stdout: &str) -> Output {
    Output {
        status: exit_status_from_code(code),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

#[cfg(unix)]
fn exit_status_from_code(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(code << 8)
}

#[cfg(windows)]
fn exit_status_from_code(code: i32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(code as u32)
}

#[test]
fn has_error_with_non_zero_exit_code() {
    assert!(has_error("", Some(1)));
    assert!(has_error("anything", Some(127)));
    assert!(has_error("", Some(-1)));
}

#[test]
fn has_error_with_zero_exit_code_and_no_error_patterns() {
    assert!(!has_error("Success", Some(0)));
    assert!(!has_error("completed successfully", Some(0)));
}

#[test]
fn has_error_with_error_pattern_in_output() {
    assert!(has_error("Command failed", Some(0)));
    assert!(has_error("FAILED: test suite", Some(0)));
    assert!(has_error("thread panicked", Some(0)));
}

#[test]
fn has_error_with_none_exit_code_and_no_patterns() {
    assert!(!has_error("Output without errors", None));
}

#[test]
fn has_error_with_none_exit_code_but_error_pattern() {
    assert!(has_error("command not found", None));
    assert!(has_error("segmentation fault in malloc", None));
}

#[test]
fn is_build_command_cargo() {
    assert!(is_build_command("cargo build"));
    assert!(is_build_command("cargo build --release"));
    assert!(is_build_command("cargo check"));
}

#[test]
fn is_build_command_npm_and_tsc() {
    assert!(is_build_command("npm run build"));
    assert!(is_build_command("tsc"));
    assert!(is_build_command("make"));
}

#[test]
fn is_build_command_non_build() {
    assert!(!is_build_command("ls -la"));
    assert!(!is_build_command("git status"));
    assert!(!is_build_command("echo hello"));
}

#[test]
fn is_test_command_detects_common_runners() {
    assert!(is_test_command("cargo test"));
    assert!(is_test_command("npm run test"));
    assert!(is_test_command("make test"));
    assert!(!is_test_command("cargo build"));
}

#[test]
fn successful_validation_feedback_prefers_test_commands() {
    assert_eq!(
        successful_validation_feedback("make test", Some(0)),
        Some(("test_passed", 1, "cortina.post_tool_use.test"))
    );
    assert_eq!(
        successful_validation_feedback("cargo build", Some(0)),
        Some(("build_passed", 1, "cortina.post_tool_use.build"))
    );
    assert_eq!(successful_validation_feedback("cargo test", Some(1)), None);
    assert_eq!(successful_validation_feedback("git status", Some(0)), None);
}

#[test]
fn normalize_command_multi_word() {
    assert_eq!(normalize_command("cargo build --release"), "cargo build");
    assert_eq!(
        normalize_command("cargo test --lib -- --nocapture"),
        "cargo test"
    );
}

#[test]
fn normalize_command_single_word() {
    assert_eq!(normalize_command("ls"), "ls");
    assert_eq!(normalize_command("git"), "git");
}

#[test]
fn normalize_command_empty() {
    assert_eq!(normalize_command(""), "");
}

#[test]
fn importance_as_str() {
    assert_eq!(Importance::Low.as_str(), "low");
    assert_eq!(Importance::Medium.as_str(), "medium");
    assert_eq!(Importance::High.as_str(), "high");
}

#[test]
fn temp_state_path_uses_system_temp_dir() {
    let path = temp_state_path("errors", "abc123", "json");
    assert!(path.starts_with(std::env::temp_dir()));
    assert!(path.ends_with("cortina-errors-abc123.json"));
}

#[test]
fn scope_hash_uses_explicit_cwd_when_present() {
    assert_eq!(scope_hash(Some("/tmp/demo")), scope_hash(Some("/tmp/demo")));
    assert_ne!(
        scope_hash(Some("/tmp/demo-a")),
        scope_hash(Some("/tmp/demo-b"))
    );
}

#[test]
fn project_name_for_cwd_uses_explicit_path() {
    assert_eq!(
        project_name_for_cwd(Some("/tmp/demo-project")).as_deref(),
        Some("demo-project")
    );
}

#[test]
fn ensure_hyphae_session_with_runner_leaves_state_empty_on_spawn_failure() {
    let hash = "ensure-spawn-failure";
    clear_session_state(hash);

    let state = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |_cmd| {
        Err(std::io::Error::other("spawn failed"))
    });

    assert!(state.is_none());
    assert!(load_session_state(hash).is_none());
}

#[test]
fn ensure_hyphae_session_with_runner_reuses_active_cached_state() {
    let hash = "ensure-active-state";
    clear_session_state(hash);
    let state = SessionState {
        session_id: "ses_active".to_string(),
        project: "demo-project".to_string(),
        started_at: 1,
    };
    save_json_file(session_state_path(hash), &state).unwrap();

    let mut status_calls = 0;
    let mut start_calls = 0;
    let result = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();

        match args.as_slice() {
            ["session", "status", "--id", "ses_active"] => {
                status_calls += 1;
                Ok(output_with_status(
                    0,
                    r#"{"session_id":"ses_active","project":"demo-project","scope":"ensure-active-state","status":"active","active":true}"#,
                ))
            }
            [
                "session",
                "start",
                "--project",
                "demo-project",
                "--scope",
                "ensure-active-state",
                "--task",
                "task",
            ] => {
                start_calls += 1;
                Ok(output_with_status(0, "ses_new"))
            }
            _ => panic!("unexpected hyphae command args: {args:?}"),
        }
    });

    assert_eq!(result.as_ref(), Some(&state));
    assert_eq!(status_calls, 1);
    assert_eq!(start_calls, 0);
    assert_eq!(load_session_state(hash).as_ref(), Some(&state));
    clear_session_state(hash);
}

#[test]
fn ensure_hyphae_session_with_runner_discards_stale_cached_state() {
    let hash = "ensure-stale-state";
    clear_session_state(hash);
    let stale = SessionState {
        session_id: "ses_stale".to_string(),
        project: "demo-project".to_string(),
        started_at: 1,
    };
    save_json_file(session_state_path(hash), &stale).unwrap();

    let mut status_calls = 0;
    let mut start_calls = 0;
    let result = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();

        match args.as_slice() {
            ["session", "status", "--id", "ses_stale"] => {
                status_calls += 1;
                Ok(output_with_status(
                    0,
                    r#"{"session_id":"ses_stale","project":"demo-project","scope":"ensure-stale-state","status":"completed","active":false}"#,
                ))
            }
            [
                "session",
                "start",
                "--project",
                "demo-project",
                "--scope",
                "ensure-stale-state",
                "--task",
                "task",
            ] => {
                start_calls += 1;
                Ok(output_with_status(0, "ses_fresh"))
            }
            _ => panic!("unexpected hyphae command args: {args:?}"),
        }
    });

    assert_eq!(
        result.as_ref().map(|session| session.session_id.as_str()),
        Some("ses_fresh")
    );
    assert_eq!(status_calls, 1);
    assert_eq!(start_calls, 1);
    assert_eq!(
        load_session_state(hash)
            .as_ref()
            .map(|session| session.session_id.as_str()),
        Some("ses_fresh")
    );
    clear_session_state(hash);
}

#[test]
fn ensure_hyphae_session_with_runner_ignores_other_scoped_sessions() {
    let hash = "ensure-scope-a";
    clear_session_state(hash);
    let cached = SessionState {
        session_id: "ses_scope_a".to_string(),
        project: "demo-project".to_string(),
        started_at: 1,
    };
    save_json_file(session_state_path(hash), &cached).unwrap();

    let mut status_calls = 0;
    let mut start_calls = 0;
    let result = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();

        match args.as_slice() {
            ["session", "status", "--id", "ses_scope_a"] => {
                status_calls += 1;
                Ok(output_with_status(
                    0,
                    r#"{"session_id":"ses_scope_a","project":"demo-project","scope":"ensure-scope-b","status":"active","active":true}"#,
                ))
            }
            [
                "session",
                "start",
                "--project",
                "demo-project",
                "--scope",
                "ensure-scope-a",
                "--task",
                "task",
            ] => {
                start_calls += 1;
                Ok(output_with_status(0, "ses_scope_a_fresh"))
            }
            _ => panic!("unexpected hyphae command args: {args:?}"),
        }
    });

    assert_eq!(
        result.as_ref().map(|session| session.session_id.as_str()),
        Some("ses_scope_a_fresh")
    );
    assert_eq!(status_calls, 1);
    assert_eq!(start_calls, 1);
    clear_session_state(hash);
}

#[test]
fn ensure_hyphae_session_with_runner_serializes_concurrent_starts() {
    let hash = "ensure-concurrent-start";
    clear_session_state(hash);

    let start_calls = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();

    for _ in 0..2 {
        let start_calls = Arc::clone(&start_calls);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
                let args: Vec<String> = cmd
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                let args = args.iter().map(String::as_str).collect::<Vec<_>>();

                match args.as_slice() {
                    [
                        "session",
                        "start",
                        "--project",
                        "demo-project",
                        "--scope",
                        "ensure-concurrent-start",
                        "--task",
                        "task",
                    ] => {
                        let call = start_calls.fetch_add(1, Ordering::SeqCst) + 1;
                        thread::sleep(Duration::from_millis(50));
                        Ok(output_with_status(0, &format!("ses_{call}")))
                    }
                    ["session", "status", "--id", "ses_1"] => Ok(output_with_status(
                        0,
                        r#"{"session_id":"ses_1","project":"demo-project","scope":"ensure-concurrent-start","status":"active","active":true}"#,
                    )),
                    unexpected => panic!("unexpected hyphae command args: {unexpected:?}"),
                }
            })
            .expect("session should be created")
        }));
    }

    let first = handles.remove(0).join().expect("thread should succeed");
    let second = handles.remove(0).join().expect("thread should succeed");

    assert_eq!(start_calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.session_id, "ses_1");
    assert_eq!(second.session_id, "ses_1");
    clear_session_state(hash);
}

#[test]
fn end_hyphae_session_with_missing_state_returns_false() {
    let hash = "end-missing-state";
    clear_session_state(hash);

    let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
        Ok(output_with_status(0, ""))
    });

    assert!(result.is_none());
}

#[test]
fn end_hyphae_session_with_spawn_failure_keeps_cached_state() {
    let hash = "end-spawn-failure";
    clear_session_state(hash);
    let state = SessionState {
        session_id: "ses_demo".to_string(),
        project: "demo-project".to_string(),
        started_at: 1,
    };
    save_json_file(session_state_path(hash), &state).unwrap();

    let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
        Err(std::io::Error::other("spawn failed"))
    });

    assert!(result.is_none());
    assert_eq!(load_session_state(hash).as_ref(), Some(&state));
    clear_session_state(hash);
}

#[test]
fn end_hyphae_session_with_non_zero_exit_keeps_cached_state() {
    let hash = "end-non-zero";
    clear_session_state(hash);
    let state = SessionState {
        session_id: "ses_demo".to_string(),
        project: "demo-project".to_string(),
        started_at: 1,
    };
    save_json_file(session_state_path(hash), &state).unwrap();

    let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
        Ok(output_with_status(1, "failed"))
    });

    assert!(result.is_none());
    assert_eq!(load_session_state(hash).as_ref(), Some(&state));
    clear_session_state(hash);
}

#[test]
fn end_hyphae_session_with_success_clears_cached_state() {
    let hash = "end-success";
    clear_session_state(hash);
    let state = SessionState {
        session_id: "ses_demo".to_string(),
        project: "demo-project".to_string(),
        started_at: 1,
    };
    save_json_file(session_state_path(hash), &state).unwrap();

    let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
        Ok(output_with_status(0, "ok"))
    });

    assert_eq!(result.as_ref(), Some(&state));
    assert!(load_session_state(hash).is_none());
}

#[test]
fn session_outcome_feedback_classifies_failure_keywords() {
    assert_eq!(
        session_outcome_feedback("Build failed after retries", false).3,
        SessionOutcome::Failure
    );
    assert_eq!(
        session_outcome_feedback("Work completed successfully", false).3,
        SessionOutcome::Success
    );
    assert_eq!(
        session_outcome_feedback("Work completed", true).3,
        SessionOutcome::Failure
    );
    assert_eq!(
        session_outcome_feedback("Improved error handling and validation", false).3,
        SessionOutcome::Success
    );
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct CounterState {
    value: usize,
}

#[test]
fn update_json_file_serializes_concurrent_mutations() {
    let path = temp_state_path("counter", "concurrent-update", "json");
    let _ = fs::remove_file(&path);

    let workers = 16;
    let barrier = Arc::new(Barrier::new(workers));
    let mut handles = Vec::new();

    for _ in 0..workers {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            update_json_file::<CounterState, _, _>(&path, |state| {
                state.value += 1;
            })
            .expect("counter update should succeed");
        }));
    }

    for handle in handles {
        handle.join().expect("thread should finish cleanly");
    }

    let state: CounterState = load_json_file(&path).expect("counter state should exist");
    assert_eq!(state.value, workers);

    let _ = fs::remove_file(path);
}

#[test]
fn update_json_file_recovers_stale_lock() {
    let path = temp_state_path("counter", "stale-lock", "json");
    let lock_path = lock_path_for(&path);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&lock_path);
    fs::write(
        &lock_path,
        format!(
            "stale-owner {}\n",
            current_timestamp_ms().saturating_sub(30_000 + 1_000)
        ),
    )
    .unwrap();

    update_json_file::<CounterState, _, _>(&path, |state| {
        state.value = 7;
    })
    .expect("stale lock should be recovered");

    let state: CounterState = load_json_file(&path).expect("counter state should exist");
    assert_eq!(state.value, 7);

    let _ = fs::remove_file(path);
    let _ = fs::remove_file(lock_path);
}

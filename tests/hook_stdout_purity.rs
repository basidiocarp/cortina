use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Minimal Claude Code pre-tool-use envelope accepted by cortina.
/// Uses "Write" tool which triggers write_advisory -> scope_hash -> git subprocess.
const SYNTHETIC_PRE_TOOL_USE_TEMPLATE: &str = r#"{
    "tool_name": "Write",
    "tool_input": {"file_path": "{git_dir}/test.rs"},
    "session_id": "test-session",
    "cwd": "{git_dir}"
}"#;

#[test]
fn cortina_pre_tool_use_stdout_is_clean_json_or_empty() {
    // Create a temporary git repository so that scope_hash actually invokes git commands
    // and produces real output that would leak if stdout is not piped.
    let temp_dir = tempfile::Builder::new()
        .prefix("cortina-test-")
        .tempdir()
        .expect("create temp dir");
    let temp_path = temp_dir.path();

    // Initialize a git repo
    Command::new("git")
        .args(["init"])
        .current_dir(temp_path)
        .output()
        .expect("git init should succeed");

    // Configure git user for the test repo
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output()
        .ok();

    // Build the synthetic pre-tool-use event with the temp git directory
    let git_dir_str = temp_path.to_string_lossy();
    let synthetic_input = SYNTHETIC_PRE_TOOL_USE_TEMPLATE
        .replace("{git_dir}", &git_dir_str);

    let binary = env!("CARGO_BIN_EXE_cortina");

    let mut child = Command::new(binary)
        .args(["adapter", "claude-code", "pre-tool-use"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cortina binary should spawn");

    // Write the synthetic payload to stdin.
    use std::io::Write;
    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        stdin.write_all(synthetic_input.as_bytes())
            .expect("write synthetic payload to cortina stdin");
    }

    // Wait for cortina to exit with a 30-second timeout.
    let (tx, rx) = mpsc::channel();
    let _handle = std::thread::spawn(move || {
        tx.send(child.wait_with_output()).ok();
    });

    let output = rx.recv_timeout(Duration::from_secs(30))
        .expect("cortina timed out after 30s — possible hang in hook handler")
        .expect("cortina should exit cleanly");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Invariant: hook stdout must be empty or valid JSON.
    // Any non-JSON content (git log lines, debug output, etc.) is a bug.
    if !stdout.trim().is_empty() {
        serde_json::from_str::<serde_json::Value>(stdout.trim())
            .unwrap_or_else(|_| panic!(
                "cortina pre-tool-use stdout is not valid JSON:\n{stdout}"
            ));
    }

    // temp_dir is dropped here and cleaned up automatically.
}

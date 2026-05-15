// HookExecutor and related types are infrastructure for a future multi-hook dispatch system.
// They are tested in this module but not yet wired into main().

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use super::types::{HookInput, HookOutput, HookType};

#[allow(
    dead_code,
    reason = "HookExecutor is infrastructure for future multi-hook dispatch"
)]
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);
#[allow(
    dead_code,
    reason = "HookExecutor is infrastructure for future multi-hook dispatch"
)]
const CONTEXT_MOD_LIMIT: usize = 50 * 1024; // 50KB

/// Executor for hook processes.
///
/// Runs hook executables found in configured directories, spawning each as a
/// subprocess with input serialized to JSON on stdin. Enforces a 30-second
/// timeout per hook and aggregates outputs across all matching hooks.
#[allow(
    dead_code,
    reason = "HookExecutor is infrastructure for future multi-hook dispatch"
)]
pub struct HookExecutor {
    /// Directories to search for hook executables.
    pub hooks_dirs: Vec<PathBuf>,
}

impl HookExecutor {
    /// Create a new hook executor with the given hook directories.
    #[must_use]
    #[allow(
        dead_code,
        reason = "HookExecutor is infrastructure for future multi-hook dispatch"
    )]
    pub fn new(hooks_dirs: Vec<PathBuf>) -> Self {
        HookExecutor { hooks_dirs }
    }

    /// Run all hooks matching the given hook type.
    ///
    /// Searches `hooks_dirs` for executables named after `hook_type` (`snake_case`),
    /// runs each with input serialized to JSON on stdin, and aggregates outputs.
    ///
    /// Exit code mapping:
    /// - exit 0 → `HookOutput::default()` (allow)
    /// - exit 2 → `HookOutput { cancel: true, .. }` (block tool)
    /// - exit 49 → `HookOutput { halt_turn: true, .. }` (halt turn)
    /// - any other non-zero → log warning, return default (fail-open)
    ///
    /// Timeout, spawn error, or JSON parse failure always return default (fail-open).
    /// If multiple hooks match, outputs are merged: `cancel` and `halt_turn` are OR'd;
    /// `context_modification` from the last hook that sets it wins;
    /// `error` messages are concatenated.
    #[allow(
        dead_code,
        reason = "HookExecutor is infrastructure for future multi-hook dispatch"
    )]
    pub fn run_hooks(&self, hook_type: HookType, input: &HookInput) -> HookOutput {
        let hook_name = format!("{hook_type:?}")
            .chars()
            .fold(String::new(), |mut acc, c| {
                if c.is_uppercase() && !acc.is_empty() {
                    acc.push('_');
                    acc.push(c.to_ascii_lowercase());
                } else {
                    acc.push(c.to_ascii_lowercase());
                }
                acc
            });

        let mut aggregated_output = HookOutput::default();
        let input_json = match serde_json::to_string(input) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("Warning: failed to serialize hook input: {e}");
                return HookOutput::default();
            }
        };

        for hooks_dir in &self.hooks_dirs {
            if !hooks_dir.is_dir() {
                continue;
            }

            let Ok(entries) = fs::read_dir(hooks_dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = match path.file_name() {
                    Some(n) => n.to_string_lossy().to_string(),
                    None => continue,
                };

                if file_name != hook_name {
                    continue;
                }

                // Execute the hook
                let output = self.execute_hook(&path, &input_json);
                self.merge_outputs(&mut aggregated_output, output);
            }
        }

        aggregated_output
    }

    #[allow(
        clippy::unused_self,
        clippy::uninlined_format_args,
        dead_code,
        reason = "HookExecutor is infrastructure for future multi-hook dispatch"
    )]
    fn execute_hook(&self, hook_path: &PathBuf, input_json: &str) -> HookOutput {
        let mut child = match Command::new(hook_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "Warning: failed to spawn hook {}: {}",
                    hook_path.display(),
                    e
                );
                return HookOutput::default();
            }
        };

        // Write input to stdin
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(input_json.as_bytes()) {
                eprintln!(
                    "Warning: failed to write to hook stdin {}: {}",
                    hook_path.display(),
                    e
                );
                return HookOutput::default();
            }
            drop(stdin);
        }

        // Use channel-based timeout pattern instead of busy-poll
        let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<std::process::Output>>();
        thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });

        let output = match rx.recv_timeout(HOOK_TIMEOUT) {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                eprintln!("Warning: hook {} wait failed: {}", hook_path.display(), e);
                return HookOutput::default();
            }
            Err(_) => {
                eprintln!("Warning: hook {} timed out", hook_path.display());
                return HookOutput::default();
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        match exit_code {
            0 => {
                // Parse JSON output
                match serde_json::from_str::<HookOutput>(stdout.trim()) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!(
                            "Warning: hook {} produced invalid JSON: {}",
                            hook_path.display(),
                            e
                        );
                        HookOutput::default()
                    }
                }
            }
            2 => HookOutput {
                cancel: true,
                ..Default::default()
            },
            49 => HookOutput {
                halt_turn: true,
                ..Default::default()
            },
            _ => {
                eprintln!(
                    "Warning: hook {} exited with code {}: {}",
                    hook_path.display(),
                    exit_code,
                    stderr
                );
                HookOutput::default()
            }
        }
    }

    #[allow(
        clippy::unused_self,
        dead_code,
        reason = "HookExecutor is infrastructure for future multi-hook dispatch"
    )]
    fn merge_outputs(&self, aggregated: &mut HookOutput, new_output: HookOutput) {
        aggregated.cancel = aggregated.cancel || new_output.cancel;
        aggregated.halt_turn = aggregated.halt_turn || new_output.halt_turn;

        if let Some(new_mod) = new_output.context_modification {
            let json_str = new_mod.to_string();
            if json_str.len() <= CONTEXT_MOD_LIMIT {
                aggregated.context_modification = Some(new_mod);
            } else {
                eprintln!(
                    "Warning: context modification exceeds {CONTEXT_MOD_LIMIT} byte limit, discarding"
                );
            }
        }

        if let Some(new_error) = new_output.error {
            match &mut aggregated.error {
                Some(existing) => {
                    existing.push('\n');
                    existing.push_str(&new_error);
                }
                None => aggregated.error = Some(new_error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_hook_executor() {
        let executor = HookExecutor::new(vec![]);
        assert!(executor.hooks_dirs.is_empty());
    }

    #[test]
    fn empty_dirs_succeeds_silently() {
        let executor = HookExecutor::new(vec![]);
        let input = HookInput {
            hook_type: HookType::PreToolUse,
            tool_name: Some("test_tool".to_string()),
            context: serde_json::json!({}),
        };
        let output = executor.run_hooks(HookType::PreToolUse, &input);
        assert!(!output.cancel);
        assert!(output.context_modification.is_none());
        assert!(output.error.is_none());
    }

    #[test]
    fn default_hook_output_is_pass_through() {
        let output = HookOutput::default();
        assert!(!output.cancel);
        assert!(output.context_modification.is_none());
        assert!(output.error.is_none());
        assert!(!output.halt_turn);
    }

    #[test]
    fn hook_output_halt_turn_field_defaults_to_false() {
        let output = HookOutput {
            cancel: false,
            context_modification: None,
            error: None,
            halt_turn: false,
        };
        assert!(!output.halt_turn);
    }

    #[test]
    fn hook_output_can_signal_halt_turn() {
        let output = HookOutput {
            cancel: false,
            context_modification: None,
            error: None,
            halt_turn: true,
        };
        assert!(output.halt_turn);
        assert!(!output.cancel);
    }

    #[test]
    #[cfg(unix)]
    fn hook_exit_0_returns_pass_through() {
        use tempfile::TempDir;

        let hooks_dir = TempDir::new().expect("create temp dir");
        let hook_path = hooks_dir.path().join("pre_tool_use");

        let script = r#"#!/bin/sh
cat << 'EOF'
{"cancel": false, "context_modification": null, "error": null, "halt_turn": false}
EOF
"#;

        std::fs::write(&hook_path, script).expect("write hook script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))
                .expect("set executable");
        }

        let executor = HookExecutor::new(vec![hooks_dir.path().to_path_buf()]);
        let input = HookInput {
            hook_type: HookType::PreToolUse,
            tool_name: Some("test_tool".to_string()),
            context: serde_json::json!({}),
        };

        let output = executor.run_hooks(HookType::PreToolUse, &input);
        assert!(!output.cancel);
        assert!(!output.halt_turn);
        assert!(output.error.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn hook_exit_2_signals_cancel() {
        use tempfile::TempDir;

        let hooks_dir = TempDir::new().expect("create temp dir");
        let hook_path = hooks_dir.path().join("pre_tool_use");

        let script = r#"#!/bin/sh
exit 2
"#;

        std::fs::write(&hook_path, script).expect("write hook script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))
                .expect("set executable");
        }

        let executor = HookExecutor::new(vec![hooks_dir.path().to_path_buf()]);
        let input = HookInput {
            hook_type: HookType::PreToolUse,
            tool_name: Some("test_tool".to_string()),
            context: serde_json::json!({}),
        };

        let output = executor.run_hooks(HookType::PreToolUse, &input);
        assert!(output.cancel);
        assert!(!output.halt_turn);
    }
}

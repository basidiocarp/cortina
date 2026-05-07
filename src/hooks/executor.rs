use std::path::PathBuf;
use std::time::Duration;

use super::types::{HookInput, HookOutput, HookType};

#[allow(dead_code)] // Part of stub API design
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);
#[allow(dead_code)] // Part of stub API design
const CONTEXT_MOD_LIMIT: usize = 50 * 1024; // 50KB

/// Executor for hook processes.
///
/// **Stub implementation.** This executor is a no-op placeholder — no hook
/// subprocesses are launched or awaited. The API is in place; real execute
/// behavior (subprocess spawn, 30-second timeout, stdout/stderr aggregation,
/// fail-open diagnostics on nonzero exit) is follow-on work.
///
/// If no hooks are configured, or until real execution is wired, execution
/// proceeds silently and fail-open.
#[allow(dead_code)] // Stub implementation for follow-on work
pub struct HookExecutor {
    /// Directories to search for hook executables.
    pub hooks_dirs: Vec<PathBuf>,
}

#[allow(dead_code)] // Stub implementation for follow-on work
impl HookExecutor {
    /// Create a new hook executor with the given hook directories.
    #[must_use]
    pub fn new(hooks_dirs: Vec<PathBuf>) -> Self {
        HookExecutor { hooks_dirs }
    }

    /// Run all hooks matching the given hook type.
    ///
    /// **Stub.** Returns the default pass-through `HookOutput` without
    /// searching directories or launching subprocesses.
    ///
    /// When real execution is implemented, this will:
    /// - search `hooks_dirs` for executables named after `hook_type`
    /// - run each with input serialized to JSON on stdin
    /// - enforce a 30-second timeout per hook
    /// - aggregate outputs across all hooks
    /// - map exit codes to signals:
    ///   - exit 0 → `HookOutput::default()` (allow)
    ///   - exit 2 → `HookOutput { cancel: true, .. }` (block tool)
    ///   - exit 49 → `HookOutput { halt_turn: true, .. }` (halt turn)
    /// - log a diagnostic warning and continue on nonzero exit or timeout
    pub fn run_hooks(&self, hook_type: HookType, input: &HookInput) -> HookOutput {
        // Stub: no hook processes to run yet; returns fail-open default
        let _ = (hook_type, input, &self.hooks_dirs); // suppress unused warnings
        HookOutput::default()
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
}

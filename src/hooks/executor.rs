use std::path::PathBuf;
use std::time::Duration;

use super::types::{HookInput, HookOutput, HookType};

#[allow(dead_code)] // Part of stub API design
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);
#[allow(dead_code)] // Part of stub API design
const CONTEXT_MOD_LIMIT: usize = 50 * 1024; // 50KB

/// Executor for hook processes.
///
/// The executor loads hook executables from configured directories,
/// runs them with a 30-second timeout, and aggregates their outputs.
/// If no hooks are configured, execution proceeds silently (fail-open).
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
    /// Searches `hooks_dirs` for executables named after `hook_type`,
    /// runs them with the input serialized to JSON on stdin,
    /// enforces a 30-second timeout per hook,
    /// and aggregates outputs.
    ///
    /// If no hooks are configured or found, returns a default (pass-through)
    /// `HookOutput`. If a hook times out or fails, logs a warning and continues.
    ///
    /// This is a stub implementation — real subprocess execution is follow-on work.
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
    }
}

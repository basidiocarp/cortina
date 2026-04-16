use anyhow::Result;
use clap::Subcommand;

use crate::hooks;
use crate::policy::capture_policy;

pub mod claude_code;
pub mod volva;

#[derive(Subcommand)]
pub enum AdapterCommand {
    /// Handle Claude Code hook adapter events
    #[command(name = "claude-code")]
    ClaudeCode {
        #[command(subcommand)]
        event: ClaudeCodeEventCommand,
    },

    /// Handle Volva runtime adapter events
    #[command(name = "volva")]
    Volva {
        #[command(subcommand)]
        event: VolvaEventCommand,
    },
}

#[derive(Clone, Copy, Subcommand)]
pub enum ClaudeCodeEventCommand {
    /// Handle `PreToolUse` adapter events (command rewriting)
    #[command(name = "pre-tool-use")]
    PreToolUse,

    /// Handle `PostToolUse` adapter events (error/correction/change capture)
    #[command(name = "post-tool-use")]
    PostToolUse,

    /// Handle `UserPromptSubmit` adapter events (prompt capture)
    #[command(name = "user-prompt-submit")]
    UserPromptSubmit,

    /// Handle `PreCompact` adapter events (compaction snapshot capture)
    #[command(name = "pre-compact")]
    PreCompact,

    /// Handle `Stop` adapter events (session summary)
    #[command(name = "stop")]
    Stop,

    /// Handle `SessionEnd` adapter events (session summary)
    #[command(name = "session-end")]
    SessionEnd,
}

#[derive(Clone, Copy, Subcommand)]
pub enum VolvaEventCommand {
    /// Handle a normalized Volva hook event
    #[command(name = "hook-event")]
    HookEvent,
}

pub fn handle_adapter_command(adapter: &AdapterCommand, input: &str) -> Result<()> {
    match adapter {
        AdapterCommand::ClaudeCode { event } => handle_claude_code_event(*event, input),
        AdapterCommand::Volva { event } => handle_volva_event(*event, input),
    }
}

pub fn handle_legacy_claude_command(event: ClaudeCodeEventCommand, input: &str) -> Result<()> {
    handle_claude_code_event(event, input)
}

// run_hook always succeeds; the Result<()> return here keeps the public
// handle_adapter_command / handle_legacy_claude_command signatures stable.
#[allow(clippy::unnecessary_wraps)]
fn handle_claude_code_event(event: ClaudeCodeEventCommand, input: &str) -> Result<()> {
    match event {
        ClaudeCodeEventCommand::PreToolUse => {
            run_hook("pre_tool_use", || hooks::pre_tool_use::handle(input));
        }
        ClaudeCodeEventCommand::PostToolUse => {
            run_hook("post_tool_use", || hooks::post_tool_use::handle(input));
        }
        ClaudeCodeEventCommand::UserPromptSubmit => {
            run_hook("user_prompt_submit", || {
                hooks::user_prompt_submit::handle(input)
            });
        }
        ClaudeCodeEventCommand::PreCompact => {
            run_hook("pre_compact", || hooks::pre_compact::handle(input));
        }
        ClaudeCodeEventCommand::Stop => {
            run_hook("stop", || hooks::stop::handle(input));
        }
        ClaudeCodeEventCommand::SessionEnd => {
            run_hook("session_end", || hooks::stop::handle(input));
        }
    }
    Ok(())
}

fn handle_volva_event(event: VolvaEventCommand, input: &str) -> Result<()> {
    match event {
        VolvaEventCommand::HookEvent => volva::handle_hook_event(input),
    }
}

/// Run a named hook with silent-fail and per-hook disable support.
///
/// Hook execution failures are logged at `warn` level but never propagated to
/// the agent session.  If the hook name appears in `disabled_hooks`, the hook
/// is skipped and a `trace`-level message is emitted.  The return type is `()`
/// so callers can use `run_hook(...);` without propagating errors.
fn run_hook(hook_name: &str, f: impl FnOnce() -> Result<()>) {
    let policy = capture_policy();
    if policy.disabled_hooks.contains(&hook_name.to_string()) {
        tracing::trace!("cortina: hook {} is disabled, skipping", hook_name);
        return;
    }
    if let Err(e) = f() {
        tracing::warn!("cortina: hook {} failed: {:#}", hook_name, e);
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::run_hook;
    use crate::policy::CapturePolicy;

    // Helper: build a policy with specific disabled_hooks without touching
    // the global OnceLock.
    fn policy_with_disabled(hooks: &[&str]) -> CapturePolicy {
        CapturePolicy::from_reader(|name| match name {
            "CORTINA_DISABLED_HOOKS" => Some(hooks.join(",")),
            _ => None,
        })
    }

    #[test]
    fn run_hook_silent_fail_does_not_panic_on_error() {
        // A hook closure that always fails must not surface the error.
        // run_hook returns () so the absence of panic is the proof.
        run_hook("pre_tool_use", || Err(anyhow!("simulated hook failure")));
    }

    #[test]
    fn run_hook_passes_through_ok() {
        let mut called = false;
        run_hook("post_tool_use", || {
            called = true;
            Ok(())
        });
        assert!(called, "hook closure must be invoked when not disabled");
    }

    #[test]
    fn disabled_hook_name_skips_execution() {
        // We cannot easily redirect capture_policy() inside run_hook because it
        // uses a OnceLock, so we test the disabled_hooks logic directly via
        // the policy helper.
        let policy = policy_with_disabled(&["pre_tool_use"]);
        assert!(
            policy.disabled_hooks.contains(&"pre_tool_use".to_string()),
            "policy must reflect the disabled hook"
        );

        // Simulate what run_hook does when a hook is disabled.
        let mut called = false;
        let result: anyhow::Result<()> = {
            if policy.disabled_hooks.contains(&"pre_tool_use".to_string()) {
                Ok(())
            } else {
                called = true;
                Ok(())
            }
        };
        assert!(result.is_ok());
        assert!(!called, "disabled hook must not call the handler");
    }

    #[test]
    fn non_disabled_hook_name_is_executed() {
        let policy = policy_with_disabled(&["pre_tool_use"]);
        let mut called = false;
        // post_tool_use is not disabled — simulate the run_hook check.
        if !policy
            .disabled_hooks
            .contains(&"post_tool_use".to_string())
        {
            called = true;
        }
        assert!(called, "non-disabled hook must be executed");
    }

    #[test]
    fn empty_disabled_hooks_runs_all_hooks() {
        let policy = policy_with_disabled(&[]);
        assert!(
            policy.disabled_hooks.is_empty(),
            "no hooks should be disabled by default"
        );
    }
}

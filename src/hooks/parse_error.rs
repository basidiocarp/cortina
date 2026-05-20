use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::policy::FAIL_OPEN_LIFECYCLE_CAPTURE;

/// Parse a hook envelope or log the error and return None.
///
/// Returns `None` when parsing fails — callers should return `Ok(())` immediately.
/// Does NOT emit a stdout response; callers that need one (`PreToolUse`) must do so themselves.
pub(super) fn parse_or_allow(input: &str) -> Option<ClaudeCodeHookEnvelope> {
    match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => Some(envelope),
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            const { assert!(FAIL_OPEN_LIFECYCLE_CAPTURE) };
            None
        }
    }
}

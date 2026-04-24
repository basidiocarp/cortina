use serde::{Deserialize, Serialize};

/// Hook lifecycle event types that cortina can observe and handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookType {
    /// Pre-tool invocation hook (before a tool call executes).
    PreToolUse,
    /// Post-tool invocation hook (after a tool call completes).
    PostToolUse,
    /// Task lifecycle start event.
    TaskStart,
    /// Task lifecycle resume event.
    TaskResume,
    /// Task lifecycle cancel event.
    TaskCancel,
    /// Task lifecycle complete event.
    TaskComplete,
    /// Notification event (human feedback or asynchronous updates).
    Notification,
    /// Pre-compact hook (before session context compression).
    PreCompact,
}

/// Input provided to a hook handler.
///
/// The `hook_type` identifies which lifecycle event triggered the hook.
/// `tool_name` is set for `PreToolUse` and `PostToolUse` events.
/// `context` carries the full lifecycle event data for the hook to observe.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields are part of the public API even if unused in stub.
pub struct HookInput {
    /// Which lifecycle event triggered this hook call.
    pub hook_type: HookType,
    /// Name of the tool involved (set for tool-use hooks only).
    pub tool_name: Option<String>,
    /// Full event context as JSON, passed to the hook subprocess.
    pub context: serde_json::Value,
}

/// Output returned by a hook handler.
///
/// Hooks can block execution (cancel=true), modify context payloads,
/// or return errors. By default (`cancel=false`, `context_modification=None`),
/// the outer system proceeds without change. This is the fail-open default.
#[allow(dead_code)] // Stub implementation for follow-on work
#[derive(Debug, Clone, Default)]
pub struct HookOutput {
    /// If true, the outer operation is blocked and aborted.
    /// Default: false (fail-open — errors do not block).
    pub cancel: bool,
    /// Optional modified context payload.
    /// Capped at 50KB to prevent unbounded growth.
    pub context_modification: Option<serde_json::Value>,
    /// Error message from the hook process (if any).
    pub error: Option<String>,
}

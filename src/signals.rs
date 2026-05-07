use serde::{Deserialize, Serialize};

/// The resolved signal that cortina emits after processing a hook event.
///
/// Signals communicate the outcome of hook processing to Claude Code and other
/// consumers via both exit codes and JSON response bodies.
///
/// - `Allow` → exit 0 (proceed with operation)
/// - `BlockTool` → exit 2 (block this specific tool call only)
/// - `HaltTurn` → exit 49 (halt the entire agent turn)
#[allow(dead_code)] // Part of stub API design; will be used when real hook execution is wired
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookSignal {
    /// Proceed: the outer operation is allowed to continue.
    Allow,
    /// Block: reject this specific tool call only. Exit 2 or permissionDecision=block.
    BlockTool,
    /// Halt: stop the entire agent turn. Exit 49.
    HaltTurn,
}

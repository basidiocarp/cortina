use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Kind of fact extracted from tool output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// A build/test/command error (non-zero exit or error keyword).
    Error,
    /// A git commit (detected from `git commit` output).
    Commit,
    /// A config file write (detected from file edit events).
    ConfigChange,
    /// A stated preference ("always use", "never use", "preferred").
    Preference,
}

/// A structured fact extracted from tool output by a rule-based extractor.
///
/// Consumers: hyphae (via `store_in_hyphae`), septa contract `cortina-fact-extracted-v1`.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactExtracted {
    pub kind: FactKind,
    /// Main content of the extracted fact (truncated to 500 chars).
    pub content: String,
    /// The command that produced this output, if applicable.
    pub source_command: Option<String>,
    /// Confidence in the extraction: 0.0 (very low) to 1.0 (certain).
    pub confidence: f32,
}

/// A pre-compaction snapshot emitted by Layer 1 before context compression.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreCompactSnapshot {
    /// Files actively being edited in this session.
    pub active_files: Vec<String>,
    /// Recent unresolved error summaries.
    pub open_errors: Vec<String>,
    /// Rule-based 2–3 sentence summary of what was happening. No LLM.
    pub resume_hint: String,
    /// Active task identifier, if known.
    pub active_task_id: Option<String>,
    /// Count of captured outcomes by kind label.
    pub signal_counts: HashMap<String, usize>,
}

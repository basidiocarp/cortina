pub mod executor;
pub mod fp_check;
pub mod gate_guard;
pub mod message_display;
pub mod node_context;
pub mod parse_error;
pub mod post_compact;
pub mod post_tool_use;
pub mod pre_commit;
pub mod pre_compact;
pub mod pre_tool_use;
pub mod stop;
pub mod subagent_stop;
pub mod trigger_word;
pub mod types;
pub mod user_prompt_submit;

use crate::utils::temp_state_path;
use std::path::PathBuf;

/// Path to the post-compaction recall marker for a session+worktree scope.
/// Written by the `PostCompact` hook, consumed by the next `UserPromptSubmit` so it
/// can re-recall with `--post-compaction` after context is compacted.
pub(crate) fn post_compact_marker_path(hash: &str) -> PathBuf {
    temp_state_path("post-compact-marker", hash, "json")
}

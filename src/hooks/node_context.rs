use serde::{Deserialize, Serialize};

/// Per-node hook overrides injected by hymenium when launching an agent for a DAG node.
/// Set via the `CORTINA_NODE_CONTEXT` environment variable as a JSON blob.
///
/// Node hooks take priority over session-level hooks for matched tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeHookContext {
    /// Node identifier from the workflow DAG (for logging).
    pub node_id: Option<String>,

    /// Tools explicitly allowed in this node. If non-empty, any tool NOT in this list
    /// triggers a deny. Takes priority over session-level allowed_tools.
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Tools explicitly denied in this node.
    #[serde(default)]
    pub denied_tools: Vec<String>,

    /// Pre-tool-use hook entries. Matched against `tool_name` using the `matcher` field
    /// (pipe-separated tool names or `*` for all).
    #[serde(default)]
    pub pre_tool_use: Vec<NodeHookEntry>,

    /// Post-tool-use hook entries.
    #[serde(default)]
    pub post_tool_use: Vec<NodeHookEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeHookEntry {
    /// Pipe-separated tool name pattern, e.g. `"Write|Edit"` or `"Bash"` or `"*"`.
    pub matcher: String,

    /// Context appended to the model's next turn.
    #[serde(default)]
    pub additional_context: Option<String>,

    /// System-level steering message injected at hook fire time.
    /// NOTE: Not yet wired end-to-end — field is reserved for future use.
    /// A runtime warning is emitted when this field is set.
    #[serde(default)]
    pub system_message: Option<String>,

    /// If true, deny the tool use (block, do not execute).
    #[serde(default)]
    pub deny: bool,
}

impl NodeHookContext {
    /// Returns true if `tool_name` matches any entry in `denied_tools` or any
    /// `deny: true` pre_tool_use hook entry whose matcher covers `tool_name`.
    pub fn is_denied(&self, tool_name: &str) -> bool {
        if self.denied_tools.iter().any(|t| t.trim() == tool_name) {
            return true;
        }
        self.pre_tool_use
            .iter()
            .any(|e| e.deny && matches_tool(&e.matcher, tool_name))
    }

    /// Returns true if `allowed_tools` is non-empty and `tool_name` is NOT in the list.
    pub fn is_not_allowed(&self, tool_name: &str) -> bool {
        !self.allowed_tools.is_empty() && !self.allowed_tools.iter().any(|t| t == tool_name)
    }

    /// Collect all `additional_context` strings from post_tool_use entries matching `tool_name`.
    pub fn post_tool_additional_context(&self, tool_name: &str) -> Vec<&str> {
        self.post_tool_use
            .iter()
            .filter(|e| matches_tool(&e.matcher, tool_name))
            .filter_map(|e| e.additional_context.as_deref())
            .collect()
    }
}

/// Load `NodeHookContext` from `CORTINA_NODE_CONTEXT` env var if set.
/// Returns `None` if the env var is absent; returns an error if present but invalid JSON.
pub fn load_node_context() -> anyhow::Result<Option<NodeHookContext>> {
    match std::env::var("CORTINA_NODE_CONTEXT") {
        Err(_) => Ok(None),
        Ok(raw) if raw.trim().is_empty() => Ok(None),
        Ok(raw) => {
            let ctx = serde_json::from_str::<NodeHookContext>(&raw)
                .map_err(|e| anyhow::anyhow!("CORTINA_NODE_CONTEXT is not valid JSON: {e}"))?;

            // Warn if any hook entry has system_message set (not yet implemented).
            if ctx.pre_tool_use.iter().any(|e| e.system_message.is_some())
                || ctx.post_tool_use.iter().any(|e| e.system_message.is_some())
            {
                tracing::warn!(
                    "[cortina] node hook system_message is not yet implemented (node_id: {:?})",
                    ctx.node_id
                );
            }

            Ok(Some(ctx))
        }
    }
}

/// Check if a tool name matches a matcher pattern.
/// `matcher` can be a pipe-separated list of tool names (e.g. `"Write|Edit"`),
/// a single tool name, or `"*"` to match all tools.
pub fn matches_tool(matcher: &str, tool_name: &str) -> bool {
    matcher == "*" || matcher.split('|').any(|t| t.trim() == tool_name)
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;

    #[test]
    fn is_denied_via_denied_tools() {
        let ctx = NodeHookContext {
            denied_tools: vec!["Write".to_string(), "Edit".to_string()],
            ..Default::default()
        };
        assert!(ctx.is_denied("Write"));
        assert!(ctx.is_denied("Edit"));
        assert!(!ctx.is_denied("Read"));
    }

    #[test]
    fn is_denied_via_hook_entry() {
        let ctx = NodeHookContext {
            pre_tool_use: vec![NodeHookEntry {
                matcher: "Write|Edit".to_string(),
                deny: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(ctx.is_denied("Write"));
        assert!(ctx.is_denied("Edit"));
        assert!(!ctx.is_denied("Bash"));
    }

    #[test]
    fn post_tool_additional_context_matches_wildcard() {
        let ctx = NodeHookContext {
            post_tool_use: vec![NodeHookEntry {
                matcher: "*".to_string(),
                additional_context: Some("Run type checker now.".to_string()),
                deny: false,
                system_message: None,
            }],
            ..Default::default()
        };
        let msgs = ctx.post_tool_additional_context("Write");
        assert_eq!(msgs, vec!["Run type checker now."]);
    }

    #[test]
    fn load_node_context_absent_returns_none() {
        unsafe {
            std::env::set_var("CORTINA_NODE_CONTEXT", "");
        }
        let result = load_node_context().unwrap();
        assert!(result.is_none());
        unsafe {
            std::env::remove_var("CORTINA_NODE_CONTEXT");
        }
    }

    #[test]
    fn load_node_context_valid_json_parses() {
        let json = r#"{"node_id":"implement","denied_tools":["Write"]}"#;
        unsafe {
            std::env::set_var("CORTINA_NODE_CONTEXT", json);
        }
        let ctx = load_node_context().unwrap().unwrap();
        assert_eq!(ctx.node_id.as_deref(), Some("implement"));
        assert!(ctx.is_denied("Write"));
        unsafe {
            std::env::remove_var("CORTINA_NODE_CONTEXT");
        }
    }

    #[test]
    fn is_not_allowed_empty_means_allow_all() {
        let ctx = NodeHookContext::default();
        assert!(!ctx.is_not_allowed("Write"));
        assert!(!ctx.is_not_allowed("Bash"));
    }

    #[test]
    fn is_not_allowed_rejects_unlisted_tool() {
        let ctx = NodeHookContext {
            allowed_tools: vec!["Read".to_string()],
            ..Default::default()
        };
        assert!(ctx.is_not_allowed("Write"));
        assert!(!ctx.is_not_allowed("Read"));
    }
}

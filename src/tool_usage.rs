use crate::utils::{current_timestamp_ms, load_json_file, remove_file_with_lock, temp_state_path, update_json_file};

pub const TOOL_USAGE_STATE_NAME: &str = "tool-usage";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    Hyphae,
    Rhizome,
    Cortina,
    Mycelium,
    Canopy,
    Volva,
    Spore,
    Other,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallEntry {
    pub tool_name: String,
    pub source: ToolSource,
    pub call_count: u32,
    pub first_call_at_ms: u64,
    pub last_call_at_ms: u64,
}

fn tool_usage_state_path(hash: &str) -> std::path::PathBuf {
    temp_state_path(TOOL_USAGE_STATE_NAME, hash, "json")
}

pub fn record_tool_call(tool_name: &str, source: ToolSource, hash: &str) {
    let path = tool_usage_state_path(hash);
    let now = current_timestamp_ms();
    let _ = update_json_file::<Vec<ToolCallEntry>, _, _>(&path, |entries| {
        if let Some(entry) = entries.iter_mut().find(|e| e.tool_name == tool_name) {
            entry.call_count += 1;
            entry.last_call_at_ms = now;
        } else {
            entries.push(ToolCallEntry {
                tool_name: tool_name.to_string(),
                source,
                call_count: 1,
                first_call_at_ms: now,
                last_call_at_ms: now,
            });
        }
    });
}

pub fn load_tool_calls(hash: &str) -> Vec<ToolCallEntry> {
    let path = tool_usage_state_path(hash);
    load_json_file(path).unwrap_or_default()
}

pub fn clear_tool_calls(hash: &str) {
    let path = tool_usage_state_path(hash);
    let _ = remove_file_with_lock(&path);
}

pub fn source_for_tool(tool_name: &str) -> ToolSource {
    let lower = tool_name.to_lowercase();

    if lower.starts_with("hyphae_") || lower.starts_with("mcp__hyphae__") {
        ToolSource::Hyphae
    } else if lower.starts_with("rhizome_") || lower.starts_with("mcp__rhizome__") {
        ToolSource::Rhizome
    } else if lower.starts_with("cortina_") || lower.starts_with("mcp__cortina__") {
        ToolSource::Cortina
    } else if lower.starts_with("mycelium_") || lower.starts_with("mcp__mycelium__") {
        ToolSource::Mycelium
    } else if lower.starts_with("canopy_") || lower.starts_with("mcp__canopy__") {
        ToolSource::Canopy
    } else if lower.starts_with("volva_") || lower.starts_with("mcp__volva__") {
        ToolSource::Volva
    } else if lower.starts_with("spore_") || lower.starts_with("mcp__spore__") {
        ToolSource::Spore
    } else {
        ToolSource::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hash(test_name: &str) -> String {
        format!("test-tool-usage-{}-{}", std::process::id(), test_name)
    }

    #[test]
    fn record_tool_call_increments_count() {
        let hash = test_hash("record_tool_call_increments_count");
        let tool_name = "test_tool";
        let source = ToolSource::Other;

        // Record first call
        record_tool_call(tool_name, source.clone(), &hash);
        let calls = load_tool_calls(&hash);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, tool_name);
        assert_eq!(calls[0].call_count, 1);
        assert_eq!(calls[0].source, ToolSource::Other);

        // Record second call
        record_tool_call(tool_name, source.clone(), &hash);
        let calls = load_tool_calls(&hash);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_count, 2);

        // Cleanup
        clear_tool_calls(&hash);
    }

    #[test]
    fn source_for_tool_maps_prefixes() {
        assert_eq!(source_for_tool("hyphae_recall"), ToolSource::Hyphae);
        assert_eq!(source_for_tool("hyphae_store"), ToolSource::Hyphae);
        assert_eq!(source_for_tool("mcp__hyphae__recall"), ToolSource::Hyphae);

        assert_eq!(source_for_tool("rhizome_symbols"), ToolSource::Rhizome);
        assert_eq!(source_for_tool("mcp__rhizome__export"), ToolSource::Rhizome);

        assert_eq!(source_for_tool("cortina_signal"), ToolSource::Cortina);
        assert_eq!(source_for_tool("mycelium_proxy"), ToolSource::Mycelium);
        assert_eq!(source_for_tool("canopy_task"), ToolSource::Canopy);
        assert_eq!(source_for_tool("volva_execute"), ToolSource::Volva);
        assert_eq!(source_for_tool("spore_config"), ToolSource::Spore);

        assert_eq!(source_for_tool("unknown_tool"), ToolSource::Other);
        assert_eq!(source_for_tool("Bash"), ToolSource::Other);

        // Case insensitive
        assert_eq!(source_for_tool("HYPHAE_RECALL"), ToolSource::Hyphae);
        assert_eq!(source_for_tool("Rhizome_Symbols"), ToolSource::Rhizome);
    }

    #[test]
    fn clear_tool_calls_removes_state() {
        let hash = test_hash("clear_tool_calls_removes_state");
        let tool_name = "test_tool";
        let source = ToolSource::Other;

        record_tool_call(tool_name, source, &hash);
        let calls = load_tool_calls(&hash);
        assert!(!calls.is_empty());

        clear_tool_calls(&hash);
        let calls = load_tool_calls(&hash);
        assert!(calls.is_empty());
    }
}

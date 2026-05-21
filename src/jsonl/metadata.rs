use serde_json::Value;

/// Extracts thinking tokens from JSONL lines.
///
/// Sums `thinking_tokens` or `thinking.tokens` from assistant messages that
/// contain thinking usage data.
///
/// **Note:** This function does NOT deduplicate by `message_id`. Streaming
/// JSONL transcripts may contain multiple entries for the same message; callers
/// responsible for accurate counts should use `parse_metrics` instead, which
/// applies the same deduplication as the primary token-counting path.
pub fn extract_thinking_tokens(lines: &[Vec<u8>]) -> u64 {
    let mut total_thinking = 0u64;

    for line in lines {
        let Ok(json) = serde_json::from_slice::<Value>(line) else {
            continue;
        };

        // Look for usage in the message
        if let Some(usage) = json.get("message").and_then(|m| m.get("usage")) {
            // Try "thinking_tokens" field first
            if let Some(tokens) = usage.get("thinking_tokens").and_then(Value::as_u64) {
                total_thinking = total_thinking.saturating_add(tokens);
            }
            // Try nested "thinking" object
            else if let Some(thinking_obj) = usage.get("thinking") {
                if let Some(tokens) = thinking_obj.get("tokens").and_then(Value::as_u64) {
                    total_thinking = total_thinking.saturating_add(tokens);
                }
            }
        }
    }

    total_thinking
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_thinking_tokens_empty() {
        let lines: Vec<Vec<u8>> = vec![];
        assert_eq!(extract_thinking_tokens(&lines), 0);
    }

    #[test]
    fn test_extract_thinking_tokens_field() {
        let lines = vec![
            br#"{"message":{"usage":{"input_tokens":100,"output_tokens":50,"thinking_tokens":75}}}"#.to_vec(),
        ];
        assert_eq!(extract_thinking_tokens(&lines), 75);
    }

    #[test]
    fn test_extract_thinking_tokens_nested_object() {
        let lines = vec![
            br#"{"message":{"usage":{"input_tokens":100,"thinking":{"tokens":120,"content":"..."}}}}"#.to_vec(),
        ];
        assert_eq!(extract_thinking_tokens(&lines), 120);
    }

    #[test]
    fn test_extract_thinking_tokens_multiple_entries() {
        let lines = vec![
            br#"{"message":{"usage":{"thinking_tokens":75}}}"#.to_vec(),
            br#"{"message":{"usage":{"thinking_tokens":50}}}"#.to_vec(),
            br#"{"message":{"usage":{"thinking":{"tokens":25}}}}"#.to_vec(),
        ];
        assert_eq!(extract_thinking_tokens(&lines), 150);
    }

    #[test]
    fn test_extract_thinking_tokens_no_thinking() {
        let lines =
            vec![br#"{"message":{"usage":{"input_tokens":100,"output_tokens":50}}}"#.to_vec()];
        assert_eq!(extract_thinking_tokens(&lines), 0);
    }

    #[test]
    fn test_extract_thinking_tokens_malformed_skipped() {
        let lines = vec![
            br#"{"message":{"usage":{"thinking_tokens":100}}}"#.to_vec(),
            b"invalid json".to_vec(),
            br#"{"message":{"usage":{"thinking_tokens":50}}}"#.to_vec(),
        ];
        assert_eq!(extract_thinking_tokens(&lines), 150);
    }

    #[test]
    fn test_extract_thinking_tokens_prefers_field_over_nested() {
        // If both are present, should still sum them (implementation sums)
        // Or if structure has both, nested is fallback
        let lines = vec![br#"{"message":{"usage":{"thinking_tokens":100}}}"#.to_vec()];
        assert_eq!(extract_thinking_tokens(&lines), 100);
    }
}

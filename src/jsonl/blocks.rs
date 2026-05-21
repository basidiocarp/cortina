use serde_json::Value;

const FIVE_HOURS_MS: i64 = 5 * 60 * 60 * 1000;

/// A time-based grouping of transcript entries.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionBlock {
    pub start_ts: Option<String>,
    pub end_ts: Option<String>,
    pub token_count: u64,
}

/// Identifies session blocks based on time gaps.
///
/// Groups entries into 5-hour blocks. A gap of more than 5 hours
/// between consecutive entries starts a new block.
pub fn identify_session_blocks(lines: &[Vec<u8>]) -> Vec<SessionBlock> {
    if lines.is_empty() {
        return Vec::new();
    }

    let mut blocks = Vec::new();
    let mut current_block = SessionBlock {
        start_ts: None,
        end_ts: None,
        token_count: 0,
    };

    let mut last_timestamp: Option<i64> = None;

    for line in lines {
        let Ok(json) = serde_json::from_slice::<Value>(line) else {
            continue;
        };

        // Extract timestamp if present
        let current_timestamp = extract_timestamp(&json);

        // Check for gap
        if let (Some(last_ts), Some(current_ts)) = (last_timestamp, current_timestamp) {
            if current_ts - last_ts > FIVE_HOURS_MS {
                // Gap exceeds 5 hours, start new block
                if current_block.token_count > 0 || current_block.start_ts.is_some() {
                    blocks.push(current_block.clone());
                }
                current_block = SessionBlock {
                    start_ts: extract_timestamp_str(&json),
                    end_ts: None,
                    token_count: 0,
                };
            }
        }

        // Set start_ts if not set
        if current_block.start_ts.is_none() {
            current_block.start_ts = extract_timestamp_str(&json);
        }

        // Always update end_ts to the latest
        if let Some(ts) = extract_timestamp_str(&json) {
            current_block.end_ts = Some(ts);
        }

        // Accumulate tokens from this entry's usage. Lines are pre-filtered but not
        // deduplicated by message_id, so streaming partials for the same message may
        // overcount. token_count is a rough indicator; use parse_metrics for accurate totals.
        if let Some(usage) = json.get("message").and_then(|m| m.get("usage")) {
            if let Some(output) = usage.get("output_tokens").and_then(Value::as_u64) {
                current_block.token_count = current_block.token_count.saturating_add(output);
            }
        }

        last_timestamp = current_timestamp;
    }

    // Add final block if it has any content
    if current_block.token_count > 0 || current_block.start_ts.is_some() {
        blocks.push(current_block);
    }

    blocks
}

/// Extracts timestamp as milliseconds since epoch from JSON entry.
fn extract_timestamp(json: &Value) -> Option<i64> {
    json.get("ts")
        .and_then(Value::as_i64)
        .or_else(|| json.get("timestamp").and_then(Value::as_i64))
}

/// Extracts timestamp as ISO string from JSON entry.
fn extract_timestamp_str(json: &Value) -> Option<String> {
    json.get("ts_iso")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            json.get("timestamp_iso")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identify_session_blocks_empty() {
        let lines: Vec<Vec<u8>> = vec![];
        let blocks = identify_session_blocks(&lines);
        assert_eq!(blocks.len(), 0);
    }

    #[test]
    fn test_identify_session_blocks_single_block() {
        let jsonl = br#"{"message":{"usage":{"output_tokens":100}},"ts":1000000000000}
{"message":{"usage":{"output_tokens":50}},"ts":1000003600000}"#;
        let lines: Vec<Vec<u8>> = jsonl
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_vec())
            .collect();

        let blocks = identify_session_blocks(&lines);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].token_count, 150);
    }

    #[test]
    fn test_identify_session_blocks_two_block_gap() {
        // Gap of 6 hours (21600000 ms) between entries
        let jsonl = br#"{"message":{"usage":{"output_tokens":100}},"ts":1000000000000}
{"message":{"usage":{"output_tokens":75}},"ts":1000021600000}"#;
        let lines: Vec<Vec<u8>> = jsonl
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_vec())
            .collect();

        let blocks = identify_session_blocks(&lines);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].token_count, 100);
        assert_eq!(blocks[1].token_count, 75);
    }

    #[test]
    fn test_identify_session_blocks_no_timestamp_entries() {
        // Entries without timestamps should still be grouped
        let jsonl = br#"{"message":{"usage":{"output_tokens":50}}}
{"message":{"usage":{"output_tokens":25}}}"#;
        let lines: Vec<Vec<u8>> = jsonl
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_vec())
            .collect();

        let blocks = identify_session_blocks(&lines);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].token_count, 75);
    }

    #[test]
    fn test_identify_session_blocks_with_iso_timestamps() {
        let jsonl = br#"{"message":{"usage":{"output_tokens":100}},"ts_iso":"2024-01-01T10:00:00Z"}
{"message":{"usage":{"output_tokens":50}},"ts_iso":"2024-01-01T11:00:00Z"}"#;
        let lines: Vec<Vec<u8>> = jsonl
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_vec())
            .collect();

        let blocks = identify_session_blocks(&lines);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_ts, Some("2024-01-01T10:00:00Z".to_string()));
        assert_eq!(blocks[0].end_ts, Some("2024-01-01T11:00:00Z".to_string()));
    }

    #[test]
    fn test_identify_session_blocks_malformed_skipped() {
        let jsonl = br#"{"message":{"usage":{"output_tokens":100}},"ts":1000000000000}
not valid json
{"message":{"usage":{"output_tokens":50}},"ts":1000001800000}"#;
        let lines: Vec<Vec<u8>> = jsonl
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_vec())
            .collect();

        let blocks = identify_session_blocks(&lines);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].token_count, 150);
    }
}

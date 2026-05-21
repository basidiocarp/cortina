use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

use super::lines::read_jsonl_lines;
use super::{blocks, compaction};
use std::path::Path;

/// Aggregated token usage from a transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

impl TokenTotals {
    fn new() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }
    }
}

/// Metrics for a Claude Code JSONL transcript.
#[derive(Debug, Clone)]
pub struct TranscriptMetrics {
    pub token_totals: TokenTotals,
    pub context_pct: Option<f64>,
    pub session_blocks: Vec<crate::jsonl::SessionBlock>,
    pub compaction_count: u32,
    pub thinking_tokens: u64,
    pub speed_tokens_per_min: Option<f64>,
}

/// Parses a Claude Code JSONL transcript file and returns aggregated metrics.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn parse_transcript(path: &Path) -> Result<TranscriptMetrics> {
    let lines = read_jsonl_lines(path)?;

    let (token_totals, context_pct, thinking_tokens) = parse_metrics(&lines);
    let session_blocks = blocks::identify_session_blocks(&lines);
    let compaction_count = compaction::detect_compaction(&lines);

    Ok(TranscriptMetrics {
        token_totals,
        context_pct,
        session_blocks,
        compaction_count,
        thinking_tokens,
        speed_tokens_per_min: None,
    })
}

/// Parses token metrics from JSONL lines.
///
/// Returns `(TokenTotals, last_context_pct, thinking_tokens)`.
///
/// Claude Code JSONL has streaming entries where the same request appears
/// multiple times at different progress points. Only the FINALIZED entry
/// (has `stop_reason` field) counts. If no finalized entry exists for a
/// `message_id`, the latest in-progress partial is used.
pub fn parse_metrics(lines: &[Vec<u8>]) -> (TokenTotals, Option<f64>, u64) {
    let mut totals = TokenTotals::new();
    let mut last_context_pct: Option<f64> = None;
    let mut thinking_tokens = 0u64;

    // Track message_id -> (is_finalized, usage_data, context_pct)
    let mut message_states: HashMap<String, (bool, Value, Option<f64>)> = HashMap::new();

    for line in lines {
        let Ok(json) = serde_json::from_slice::<Value>(line) else {
            // Skip malformed JSON lines gracefully
            continue;
        };

        // Extract message ID
        let Some(message_id) = json
            .get("message")
            .and_then(|m| m.get("id"))
            .and_then(|id| id.as_str())
            .map(ToString::to_string)
        else {
            continue;
        };

        // Check if this is a finalized entry (has stop_reason)
        let is_finalized = json
            .get("message")
            .and_then(|m| m.get("stop_reason"))
            .is_some();

        // Extract usage data
        let Some(usage) = json.get("message").and_then(|m| m.get("usage")).cloned() else {
            continue;
        };

        // Extract context_pct
        let context_pct = json
            .get("context_window_percent")
            .and_then(Value::as_f64)
            .or_else(|| json.get("context_pct").and_then(Value::as_f64));

        // Update message state:
        //   - finalized entry always beats a prior partial for the same message_id
        //   - if prev was already finalized, never overwrite
        //   - if prev was partial, replace with latest (partial or finalized)
        let should_update = match message_states.get(&message_id) {
            Some((true, _, _)) => false, // already finalized — never overwrite
            None | Some((false, _, _)) => true, // prev was partial — keep latest
        };

        if should_update {
            message_states.insert(message_id, (is_finalized, usage, context_pct));
        }

        // Track last seen context_pct
        if let Some(pct) = context_pct {
            last_context_pct = Some(pct);
        }
    }

    // Sum all usage data from deduped entries (finalized or latest partial per message_id).
    // Thinking tokens are accumulated here, after dedup, so streaming duplicates don't
    // inflate the count.
    for (_, (_, usage, _)) in message_states {
        if let Some(input) = usage.get("input_tokens").and_then(Value::as_u64) {
            totals.input_tokens = totals.input_tokens.saturating_add(input);
        }
        if let Some(output) = usage.get("output_tokens").and_then(Value::as_u64) {
            totals.output_tokens = totals.output_tokens.saturating_add(output);
        }
        if let Some(cache_creation) = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
        {
            totals.cache_creation_input_tokens = totals
                .cache_creation_input_tokens
                .saturating_add(cache_creation);
        }
        if let Some(cache_read) = usage.get("cache_read_input_tokens").and_then(Value::as_u64) {
            totals.cache_read_input_tokens =
                totals.cache_read_input_tokens.saturating_add(cache_read);
        }
        if let Some(thinking_obj) = usage.get("thinking") {
            if let Some(tokens) = thinking_obj.get("tokens").and_then(Value::as_u64) {
                thinking_tokens = thinking_tokens.saturating_add(tokens);
            }
        } else if let Some(tokens) = usage.get("thinking_tokens").and_then(Value::as_u64) {
            thinking_tokens = thinking_tokens.saturating_add(tokens);
        }
    }

    (totals, last_context_pct, thinking_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_jsonl_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_parse_metrics_empty() {
        let lines: Vec<Vec<u8>> = vec![];
        let (totals, context_pct, thinking) = parse_metrics(&lines);
        assert_eq!(totals.input_tokens, 0);
        assert_eq!(totals.output_tokens, 0);
        assert_eq!(context_pct, None);
        assert_eq!(thinking, 0);
    }

    #[test]
    fn test_parse_metrics_single_finalized_entry() {
        let jsonl = br#"{"type":"assistant","message":{"id":"msg_01","model":"claude-opus-4-7","usage":{"input_tokens":1000,"output_tokens":200,"cache_creation_input_tokens":0,"cache_read_input_tokens":800},"stop_reason":"end_turn"},"context_window_percent":45.2}"#;
        let lines = vec![jsonl.to_vec()];

        let (totals, context_pct, _thinking) = parse_metrics(&lines);
        assert_eq!(totals.input_tokens, 1000);
        assert_eq!(totals.output_tokens, 200);
        assert_eq!(totals.cache_creation_input_tokens, 0);
        assert_eq!(totals.cache_read_input_tokens, 800);
        assert_eq!(context_pct, Some(45.2));
    }

    #[test]
    fn test_parse_metrics_streaming_duplicates_finalized_wins() {
        // Same message_id, streaming partial first, then finalized
        let partial = br#"{"type":"assistant","message":{"id":"msg_01","model":"claude-opus-4-7","usage":{"input_tokens":1000,"output_tokens":100}},"context_window_percent":44.0}"#;
        let finalized = br#"{"type":"assistant","message":{"id":"msg_01","model":"claude-opus-4-7","usage":{"input_tokens":1000,"output_tokens":200,"cache_creation_input_tokens":0,"cache_read_input_tokens":800},"stop_reason":"end_turn"},"context_window_percent":45.2}"#;
        let lines = vec![partial.to_vec(), finalized.to_vec()];

        let (totals, context_pct, _) = parse_metrics(&lines);
        // Should use finalized entry with 200 output tokens
        assert_eq!(totals.output_tokens, 200);
        assert_eq!(totals.cache_read_input_tokens, 800);
        assert_eq!(context_pct, Some(45.2));
    }

    #[test]
    fn test_parse_metrics_malformed_lines_skipped() {
        let good = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":100,"output_tokens":50},"stop_reason":"end_turn"}}"#;
        let bad = b"not valid json";
        let lines = vec![good.to_vec(), bad.to_vec()];

        let (totals, _, _) = parse_metrics(&lines);
        assert_eq!(totals.input_tokens, 100);
        assert_eq!(totals.output_tokens, 50);
    }

    #[test]
    fn test_parse_metrics_context_pct_alternate_field() {
        // Some entries might use "context_pct" instead of "context_window_percent"
        let jsonl = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":500,"output_tokens":100},"stop_reason":"end_turn"},"context_pct":32.5}"#;
        let lines = vec![jsonl.to_vec()];

        let (_totals, context_pct, _) = parse_metrics(&lines);
        assert_eq!(context_pct, Some(32.5));
    }

    #[test]
    fn test_parse_metrics_thinking_tokens() {
        let jsonl = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":500,"output_tokens":100,"thinking_tokens":250},"stop_reason":"end_turn"}}"#;
        let lines = vec![jsonl.to_vec()];

        let (_, _, thinking) = parse_metrics(&lines);
        assert_eq!(thinking, 250);
    }

    #[test]
    fn test_parse_metrics_thinking_tokens_not_double_counted_for_streaming_dup() {
        // Partial has 200 thinking tokens, finalized has 300. Result must be 300, not 500.
        let partial = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":1000,"output_tokens":100,"thinking_tokens":200}}}"#;
        let finalized = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":1000,"output_tokens":200,"thinking_tokens":300},"stop_reason":"end_turn"}}"#;
        let lines = vec![partial.to_vec(), finalized.to_vec()];

        let (_, _, thinking) = parse_metrics(&lines);
        assert_eq!(
            thinking, 300,
            "thinking tokens must use finalized entry only, not sum partial+finalized"
        );
    }

    #[test]
    fn test_parse_metrics_latest_partial_wins_when_no_finalized() {
        // Two consecutive partials for same message_id — latest must win.
        let partial1 = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":1000,"output_tokens":100}}}"#;
        let partial2 = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":1000,"output_tokens":150}}}"#;
        let lines = vec![partial1.to_vec(), partial2.to_vec()];

        let (totals, _, _) = parse_metrics(&lines);
        assert_eq!(
            totals.output_tokens, 150,
            "latest partial must overwrite earlier partial"
        );
    }

    #[test]
    fn test_parse_transcript_integration() {
        let content = br#"{"type":"assistant","message":{"id":"msg_01","usage":{"input_tokens":1000,"output_tokens":200,"cache_creation_input_tokens":0,"cache_read_input_tokens":800},"stop_reason":"end_turn"},"context_window_percent":45.2}
{"type":"assistant","message":{"id":"msg_02","usage":{"input_tokens":500,"output_tokens":100},"stop_reason":"end_turn"}}"#;
        let file = write_jsonl_file(content);

        let metrics = parse_transcript(file.path()).unwrap();
        assert_eq!(metrics.token_totals.input_tokens, 1500);
        assert_eq!(metrics.token_totals.output_tokens, 300);
        assert_eq!(metrics.context_pct, Some(45.2));
    }
}

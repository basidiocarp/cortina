use serde_json::Value;

/// Detects compaction events in a transcript.
///
/// A compaction event occurs when `context_window_percent` drops by > 2 points
/// between consecutive entries AND the context window size does not change
/// (to distinguish from model switches).
pub fn detect_compaction(lines: &[Vec<u8>]) -> u32 {
    if lines.is_empty() {
        return 0;
    }

    let mut compaction_count = 0u32;
    let mut last_context_pct: Option<f64> = None;
    let mut last_context_size: Option<u64> = None;

    for line in lines {
        let Ok(json) = serde_json::from_slice::<Value>(line) else {
            continue;
        };

        let current_context_pct = extract_context_percent(&json);
        let current_context_size = extract_context_size(&json);

        // Check for compaction event
        if let (Some(last_pct), Some(current_pct)) = (last_context_pct, current_context_pct) {
            let pct_drop = last_pct - current_pct;

            // Compaction: drop > 2 points AND context size unchanged
            if pct_drop > 2.0 {
                let size_unchanged = match (last_context_size, current_context_size) {
                    (Some(last_size), Some(current_size)) => last_size == current_size,
                    (None, None) => true,
                    _ => false,
                };

                if size_unchanged {
                    compaction_count = compaction_count.saturating_add(1);
                }
            }
        }

        last_context_pct = current_context_pct;
        last_context_size = current_context_size;
    }

    compaction_count
}

/// Extracts `context_window_percent` or `context_pct` from JSON entry.
fn extract_context_percent(json: &Value) -> Option<f64> {
    json.get("context_window_percent")
        .and_then(Value::as_f64)
        .or_else(|| json.get("context_pct").and_then(Value::as_f64))
}

/// Extracts context window size (if available) from JSON entry.
fn extract_context_size(json: &Value) -> Option<u64> {
    json.get("context_window_size")
        .and_then(Value::as_u64)
        .or_else(|| json.get("context_size").and_then(Value::as_u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_compaction_no_compaction() {
        let lines: Vec<Vec<u8>> = vec![
            br#"{"context_window_percent":50.0}"#.to_vec(),
            br#"{"context_window_percent":48.0}"#.to_vec(), // Drop of 2.0, not > 2.0
        ];
        assert_eq!(detect_compaction(&lines), 0);
    }

    #[test]
    fn test_detect_compaction_single_event() {
        let lines: Vec<Vec<u8>> = vec![
            br#"{"context_window_percent":50.0,"context_window_size":8192}"#.to_vec(),
            br#"{"context_window_percent":47.5,"context_window_size":8192}"#.to_vec(), // Drop of 2.5, size same
        ];
        assert_eq!(detect_compaction(&lines), 1);
    }

    #[test]
    fn test_detect_compaction_model_switch_not_counted() {
        let lines: Vec<Vec<u8>> = vec![
            br#"{"context_window_percent":50.0,"context_window_size":8192}"#.to_vec(),
            br#"{"context_window_percent":45.0,"context_window_size":32768}"#.to_vec(), // Drop of 5, but size changed
        ];
        assert_eq!(detect_compaction(&lines), 0);
    }

    #[test]
    fn test_detect_compaction_multiple_events() {
        let lines: Vec<Vec<u8>> = vec![
            br#"{"context_window_percent":100.0}"#.to_vec(),
            br#"{"context_window_percent":97.0}"#.to_vec(), // Drop of 3.0
            br#"{"context_window_percent":93.5}"#.to_vec(), // Drop of 3.5
            br#"{"context_window_percent":91.0}"#.to_vec(), // Drop of 2.5
        ];
        assert_eq!(detect_compaction(&lines), 3);
    }

    #[test]
    fn test_detect_compaction_alternate_field_name() {
        let lines: Vec<Vec<u8>> = vec![
            br#"{"context_pct":50.0}"#.to_vec(),
            br#"{"context_pct":47.0}"#.to_vec(), // Drop of 3.0
        ];
        assert_eq!(detect_compaction(&lines), 1);
    }

    #[test]
    fn test_detect_compaction_malformed_skipped() {
        let lines: Vec<Vec<u8>> = vec![
            br#"{"context_window_percent":50.0}"#.to_vec(),
            b"invalid json".to_vec(),
            br#"{"context_window_percent":47.0}"#.to_vec(), // Drop of 3.0 (comparing to first)
        ];
        // Invalid JSON is skipped, so we compare line 1 to line 3
        assert_eq!(detect_compaction(&lines), 1);
    }

    #[test]
    fn test_detect_compaction_empty() {
        let lines: Vec<Vec<u8>> = vec![];
        assert_eq!(detect_compaction(&lines), 0);
    }
}

use anyhow::Result;
use std::path::Path;

/// Reads a JSONL file and returns raw lines as byte vectors.
///
/// Performs a two-pass pre-filter: first checks each line for the `"usage":{`
/// byte pattern to skip non-usage lines cheaply before attempting full parsing.
pub fn read_jsonl_lines(path: &Path) -> Result<Vec<Vec<u8>>> {
    let content = std::fs::read(path)?;
    let mut lines = Vec::new();

    for line in content.split(|&b| b == b'\n') {
        // Skip empty lines
        if line.is_empty() {
            continue;
        }

        // Pre-filter: only include lines that contain `"usage":{` pattern
        if has_usage_field(line) {
            lines.push(line.to_vec());
        }
    }

    Ok(lines)
}

/// Checks if a line contains a `"usage":` field.
///
/// Accepts both compact (`"usage":{`) and pretty-printed (`"usage": {`) forms
/// since `serde_json`'s compact serializer emits no spaces but some tools may not.
fn has_usage_field(line: &[u8]) -> bool {
    memchr::memmem::find(line, b"\"usage\":{").is_some()
        || memchr::memmem::find(line, b"\"usage\": {").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_jsonl_lines_filters_non_usage() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        let content = b"{ \"type\":\"user\", \"message\":\"test\" }\n\
                        { \"type\":\"assistant\", \"message\":{ \"id\":\"msg_01\", \"usage\":{ \"input_tokens\":100 } } }\n\
                        { \"no_usage\":true }\n";
        file.write_all(content).unwrap();
        file.flush().unwrap();

        let lines = read_jsonl_lines(file.path()).unwrap();
        // Only the second line has "usage":{
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_read_jsonl_lines_empty_file() {
        use tempfile::NamedTempFile;

        let file = NamedTempFile::new().unwrap();
        let lines = read_jsonl_lines(file.path()).unwrap();
        assert_eq!(lines.len(), 0);
    }

    #[test]
    fn test_has_usage_field() {
        assert!(has_usage_field(br#"{ "usage":{ "input_tokens": 100 } }"#));
        assert!(has_usage_field(
            br#"{ "type": "assistant", "usage":{ "input_tokens": 200 } }"#
        ));
        assert!(!has_usage_field(br#"{ "no_usage": true }"#));
        assert!(!has_usage_field(br#"{ "data": "usage" }"#));
    }
}

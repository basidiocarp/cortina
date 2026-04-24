/// FP (false-positive) marker checker for session transcripts.
///
/// Scans session transcripts for unresolved false-positive markers
/// that indicate debugging artifacts or pending corrections.

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FpMarker {
    pub text: String,
    pub line: Option<usize>,
}

/// Processor that detects false-positive markers in transcript text.
pub struct FpCheckProcessor;

impl FpCheckProcessor {
    /// Create a new FP check processor.
    pub fn new() -> Self {
        Self
    }

    /// Scan transcript text for FP markers.
    ///
    /// Returns a summary string if any markers are found, otherwise returns None.
    /// Markers are case-sensitive and match:
    /// - `[FP]`
    /// - `FP:`
    /// - `FALSE_POSITIVE:`
    pub fn process(transcript_text: &str) -> Option<String> {
        let markers = Self::scan(transcript_text);
        if markers.is_empty() {
            return None;
        }

        tracing::info!("cortina stop: FP markers found: {}", markers.len());
        Some(format!("FP markers found: {}", markers.len()))
    }

    /// Internal: scan and collect FP markers.
    fn scan(text: &str) -> Vec<FpMarker> {
        let mut markers = Vec::new();

        for (line_num, line) in text.lines().enumerate() {
            if line.contains("[FP]") || line.contains("FP:") || line.contains("FALSE_POSITIVE:") {
                markers.push(FpMarker {
                    text: line.trim().to_string(),
                    line: Some(line_num + 1),
                });
            }
        }

        markers
    }
}

impl Default for FpCheckProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_markers_returns_none() {
        let text = "This is a normal transcript\nwith no markers\nin it.";
        assert_eq!(FpCheckProcessor::process(text), None);
    }

    #[test]
    fn finds_fp_bracket_marker() {
        let text = "Some line\n[FP] This is flagged\nAnother line";
        let result = FpCheckProcessor::process(text);
        assert!(result.is_some());
        assert!(result.unwrap().contains('1'));
    }

    #[test]
    fn finds_fp_colon_marker() {
        let text = "Line one\nFP: Something is wrong\nLine three";
        let result = FpCheckProcessor::process(text);
        assert!(result.is_some());
        assert!(result.unwrap().contains('1'));
    }

    #[test]
    fn finds_false_positive_marker() {
        let text = "Start\nFALSE_POSITIVE: marker here\nEnd";
        let result = FpCheckProcessor::process(text);
        assert!(result.is_some());
        assert!(result.unwrap().contains('1'));
    }

    #[test]
    fn finds_multiple_markers() {
        let text = "Line\n[FP] first\nLine\nFP: second\nLine\nFALSE_POSITIVE: third";
        let result = FpCheckProcessor::process(text);
        assert!(result.is_some());
        assert!(result.unwrap().contains('3'));
    }

    #[test]
    fn case_sensitive_matching() {
        let text = "Line with fp: lowercase\nand FP: uppercase\n";
        let result = FpCheckProcessor::process(text);
        // Only uppercase FP: should match
        assert!(result.is_some());
        assert!(result.unwrap().contains('1'));
    }
}

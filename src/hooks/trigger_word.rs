/// Trigger word processor for extracting and storing inline memories.
///
/// Scans the final assistant message for keyword prefixes (e.g., `"MEMORIZE:"`, `"HYPHAE_STORE"`)
/// and stores the content via the hyphae CLI.

#[derive(Debug, Clone)]
pub struct TriggerWordPayload {
    pub keyword: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct TriggerWordConfig {
    pub keywords: Vec<String>,
}

impl Default for TriggerWordConfig {
    fn default() -> Self {
        Self {
            keywords: vec!["MEMORIZE".to_string(), "HYPHAE_STORE".to_string()],
        }
    }
}

/// Processor that scans for trigger words in final assistant messages.
pub struct TriggerWordProcessor {
    config: TriggerWordConfig,
}

impl TriggerWordProcessor {
    /// Create a new trigger word processor with the given config.
    pub fn new(config: TriggerWordConfig) -> Self {
        Self { config }
    }

    /// Scan a message for trigger word prefixes.
    ///
    /// Returns a vector of extracted payloads.
    /// Scanning is case-sensitive: looks for lines starting with `{KEYWORD}:`
    /// where KEYWORD is in the config.keywords list.
    pub fn scan(&self, message: &str) -> Vec<TriggerWordPayload> {
        let mut payloads = Vec::new();

        for line in message.lines() {
            let trimmed = line.trim();
            for keyword in &self.config.keywords {
                let prefix = format!("{keyword}:");
                if let Some(content) = trimmed.strip_prefix(&prefix) {
                    payloads.push(TriggerWordPayload {
                        keyword: keyword.clone(),
                        content: content.trim().to_string(),
                    });
                    break; // Don't match the same line twice
                }
            }
        }

        payloads
    }

    /// Store payloads via hyphae CLI.
    ///
    /// Attempts to call `hyphae store` for each payload.
    /// If hyphae is unavailable or the command fails, logs a warning
    /// and continues without blocking.
    pub fn store(payloads: &[TriggerWordPayload]) {
        for payload in payloads {
            if let Err(e) = Self::store_single(payload) {
                tracing::warn!("cortina: failed to store trigger word memory: {e}");
            }
        }
    }

    /// Store a single payload via hyphae.
    fn store_single(payload: &TriggerWordPayload) -> std::io::Result<()> {
        use std::process::Command;

        // Topic is always context/inline for trigger words
        const TOPIC: &str = "context/inline";

        // Call: hyphae store <topic> <content>
        let output = Command::new("hyphae")
            .args(["store", TOPIC, &payload.content])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(std::io::Error::other(format!(
                "hyphae store failed: {stderr}"
            )));
        }

        tracing::info!(
            "cortina: stored trigger word memory for {}: {}",
            payload.keyword,
            payload.content
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_keywords() {
        let config = TriggerWordConfig::default();
        assert_eq!(config.keywords.len(), 2);
        assert!(config.keywords.contains(&"MEMORIZE".to_string()));
        assert!(config.keywords.contains(&"HYPHAE_STORE".to_string()));
    }

    #[test]
    fn scan_finds_no_keywords() {
        let processor = TriggerWordProcessor::new(TriggerWordConfig::default());
        let message = "This is a normal message\nwith no keywords";
        let payloads = processor.scan(message);
        assert!(payloads.is_empty());
    }

    #[test]
    fn scan_finds_memorize_keyword() {
        let processor = TriggerWordProcessor::new(TriggerWordConfig::default());
        let message = "Some text\nMEMORIZE: important context here\nMore text";
        let payloads = processor.scan(message);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].keyword, "MEMORIZE");
        assert_eq!(payloads[0].content, "important context here");
    }

    #[test]
    fn scan_finds_hyphae_store_keyword() {
        let processor = TriggerWordProcessor::new(TriggerWordConfig::default());
        let message = "Text\nHYPHAE_STORE: store this\nMore";
        let payloads = processor.scan(message);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].keyword, "HYPHAE_STORE");
        assert_eq!(payloads[0].content, "store this");
    }

    #[test]
    fn scan_finds_multiple_keywords() {
        let processor = TriggerWordProcessor::new(TriggerWordConfig::default());
        let message = "MEMORIZE: first\nHYPHAE_STORE: second\nMEMORIZE: third";
        let payloads = processor.scan(message);
        assert_eq!(payloads.len(), 3);
        assert_eq!(payloads[0].keyword, "MEMORIZE");
        assert_eq!(payloads[1].keyword, "HYPHAE_STORE");
        assert_eq!(payloads[2].keyword, "MEMORIZE");
    }

    #[test]
    fn scan_case_sensitive() {
        let processor = TriggerWordProcessor::new(TriggerWordConfig::default());
        let message = "memorize: lowercase\nMEMORIZE: uppercase\nhyphae_store: wrong case";
        let payloads = processor.scan(message);
        // Only MEMORIZE and HYPHAE_STORE should match (uppercase)
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].keyword, "MEMORIZE");
    }

    #[test]
    fn scan_ignores_keyword_in_middle_of_line() {
        let processor = TriggerWordProcessor::new(TriggerWordConfig::default());
        let message = "Some text MEMORIZE: this should not match";
        let payloads = processor.scan(message);
        // Keyword must be at the start of the line (after trimming)
        assert_eq!(payloads.len(), 0);
    }

    #[test]
    fn scan_handles_whitespace() {
        let processor = TriggerWordProcessor::new(TriggerWordConfig::default());
        let message = "  MEMORIZE:  content with spaces  ";
        let payloads = processor.scan(message);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].content, "content with spaces");
    }

    #[test]
    fn custom_config_respects_keywords() {
        let config = TriggerWordConfig {
            keywords: vec!["CUSTOM".to_string()],
        };
        let processor = TriggerWordProcessor::new(config);
        let message = "MEMORIZE: should not match\nCUSTOM: should match";
        let payloads = processor.scan(message);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].keyword, "CUSTOM");
    }
}

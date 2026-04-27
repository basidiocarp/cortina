/// Redacts secrets and sensitive information from strings.
///
/// This function replaces secret-like patterns with placeholder text to prevent
/// credentials from being logged or stored. It targets:
/// - API keys and tokens (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc.)
/// - Bearer tokens
/// - Authorization headers
/// - Generic secrets and passwords
pub fn redact_secrets(s: &str) -> String {
    let mut result = s.to_string();

    // Redact in this order to handle overlapping patterns
    result = redact_key_assignments(&result);
    result = redact_bearer_tokens(&result);
    result = redact_authorization_header(&result);
    result = redact_aws_keys(&result);
    result = redact_url_credentials(&result);

    result
}

/// Redacts key=value or key: value assignments.
fn redact_key_assignments(text: &str) -> String {
    let keys = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "API_KEY",
        "SECRET",
        "PASSWORD",
        "TOKEN",
        "SK_LIVE",
        "SK_TEST",
    ];

    let mut result = text.to_string();

    for key in &keys {
        result = redact_single_key_assignment(&result, key);
    }

    result
}

/// Redacts a single key assignment.
fn redact_single_key_assignment(text: &str, key_name: &str) -> String {
    let mut result = String::new();
    let key_lower = key_name.to_lowercase();

    for line in text.lines() {
        let line_lower = line.to_lowercase();

        // Check if this line contains the key
        if let Some(key_pos) = line_lower.find(&key_lower) {
            let after_key = &line[key_pos + key_name.len()..];
            let after_key_lower = after_key.to_lowercase();

            // Look for = or : after the key
            if let Some(eq_pos) = after_key_lower.find('=') {
                result.push_str(&line[..key_pos + key_name.len() + eq_pos]);
                result.push('=');
                result.push_str("[REDACTED]");
                let remaining = &after_key[eq_pos + 1..];
                // Find next space
                if let Some(space_pos) = remaining.find(' ') {
                    result.push_str(&remaining[space_pos..]);
                }
                result.push('\n');
                continue;
            } else if let Some(colon_pos) = after_key_lower.find(':') {
                result.push_str(&line[..key_pos + key_name.len() + colon_pos]);
                result.push(':');
                result.push_str(" [REDACTED]");
                let remaining = &after_key[colon_pos + 1..];
                // Skip whitespace after colon
                let trimmed_remaining = remaining.trim_start();
                if trimmed_remaining.is_empty() {
                    result.push('\n');
                    continue;
                }
                // Find next space to preserve rest of line
                if let Some(space_pos) = trimmed_remaining.find(' ') {
                    result.push_str(&trimmed_remaining[space_pos..]);
                }
                result.push('\n');
                continue;
            }
        }

        result.push_str(line);
        result.push('\n');
    }

    // Remove trailing newline if original text didn't have it
    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Redacts bearer token values.
fn redact_bearer_tokens(text: &str) -> String {
    let mut result = String::new();

    for line in text.lines() {
        if line.to_lowercase().contains("bearer") {
            let mut chars = line.chars().peekable();
            let mut current = String::new();
            let mut in_bearer = false;

            while let Some(ch) = chars.next() {
                if !in_bearer {
                    current.push(ch);
                    if current.to_lowercase().ends_with("bearer") {
                        in_bearer = true;
                        result.push_str(&current);
                        current.clear();
                        // Skip whitespace after bearer
                        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
                            chars.next();
                        }
                        result.push_str(" [REDACTED]");
                    }
                } else if ch == ' ' || ch == '\t' || ch == '\n' {
                    in_bearer = false;
                    result.push(ch);
                } else {
                    // Skip token characters
                }
            }
            result.push_str(&current);
            result.push('\n');
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing newline if original didn't have it
    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Redacts Authorization header values.
fn redact_authorization_header(text: &str) -> String {
    let mut result = String::new();

    for line in text.lines() {
        if line.to_lowercase().contains("authorization") {
            if let Some(colon_pos) = line.find(':') {
                result.push_str(&line[..=colon_pos]);
                result.push_str(" [REDACTED]");
                result.push('\n');
            } else {
                result.push_str(line);
                result.push('\n');
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing newline if original didn't have it
    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Redacts AWS-style keys (AKIA prefix).
fn redact_aws_keys(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if (ch == 'A' || ch == 'a') && chars.peek() == Some(&'K') {
            let mut potential_key = String::from(ch);
            let mut temp_chars = chars.clone();

            // Try to match AKIA + 16 chars (total 20)
            for _ in 0..19 {
                if let Some(next_ch) = temp_chars.next() {
                    potential_key.push(next_ch);
                } else {
                    break;
                }
            }

            if potential_key.to_uppercase().starts_with("AKIA")
                && potential_key.len() == 20
                && potential_key[4..].chars().all(char::is_alphanumeric)
            {
                // Skip the matched characters in the main iterator
                for _ in 0..19 {
                    chars.next();
                }
                result.push_str("[REDACTED]");
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Redacts credentials in URLs (e.g. `https://user:pass@host`).
fn redact_url_credentials(text: &str) -> String {
    let mut result = String::new();

    for line in text.lines() {
        if (line.contains("http://") || line.contains("https://")) && line.contains('@') {
            // Use byte-safe find() to locate the protocol boundary.
            let proto_end = line
                .find("https://")
                .map(|i| i + 8)
                .or_else(|| line.find("http://").map(|i| i + 7));

            if let Some(proto_end) = proto_end {
                if let Some(at_offset) = line[proto_end..].find('@') {
                    let at_pos = proto_end + at_offset;
                    result.push_str(&line[..proto_end]);
                    result.push_str("[REDACTED]:[REDACTED]@");
                    result.push_str(&line[at_pos + 1..]);
                    result.push('\n');
                    continue;
                }
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    // Remove trailing newline if original didn't have it
    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_anthropic_api_key_with_equals() {
        let input = "ANTHROPIC_API_KEY=sk-ant-1234567890abcdef";
        let output = redact_secrets(input);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("sk-ant-1234567890abcdef"));
    }

    #[test]
    fn redacts_openai_api_key_with_colon() {
        let input = "OPENAI_API_KEY: sk-proj-1234567890abcdef";
        let output = redact_secrets(input);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("sk-proj-1234567890abcdef"));
    }

    #[test]
    fn redacts_bearer_tokens() {
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let output = redact_secrets(input);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("eyJhbGc"));
    }

    #[test]
    fn redacts_authorization_header() {
        let input = "Authorization: Basic dXNlcm5hbWU6cGFzc3dvcmQ=";
        let output = redact_secrets(input);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("dXNlcm5hbWU6cGFzc3dvcmQ="));
    }

    #[test]
    fn redacts_aws_keys() {
        let input = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        let output = redact_secrets(input);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn leaves_clean_text_unchanged() {
        let input = "This is a clean log message with no secrets";
        let output = redact_secrets(input);
        assert_eq!(output, input);
    }

    #[test]
    fn redacts_multiple_secrets_in_same_input() {
        let input = "API_KEY=secret123 and PASSWORD=mysecret";
        let output = redact_secrets(input);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret123"));
        assert!(!output.contains("mysecret"));
    }

    #[test]
    fn redacts_case_insensitive_tokens() {
        let input = "api_key=secret123";
        let output = redact_secrets(input);
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret123"));
    }

    #[test]
    fn preserves_non_secret_content() {
        let input = "Running command: cargo test\nAPI_KEY=secret123\nTest passed";
        let output = redact_secrets(input);
        assert!(output.contains("cargo test"));
        assert!(output.contains("Test passed"));
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret123"));
    }
}

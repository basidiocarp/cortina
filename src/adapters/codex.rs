use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::utils::{Importance, project_name_for_cwd, store_in_hyphae};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexTurnPayload {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_number: Option<u32>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[allow(clippy::unnecessary_wraps)]
pub fn handle_turn_complete(input: &str) -> Result<()> {
    let payload = match serde_json::from_str::<CodexTurnPayload>(input) {
        Ok(p) => p,
        Err(e) => {
            warn!("cortina: failed to parse codex turn-complete payload: {e}");
            CodexTurnPayload::default()
        }
    };

    let project = payload
        .cwd
        .as_deref()
        .and_then(|cwd| project_name_for_cwd(Some(cwd)));

    let signal = serde_json::json!({
        "host": "codex",
        "session_id": payload.session_id,
        "turn_number": payload.turn_number,
        "model": payload.model,
        "cwd": payload.cwd,
        "cost_usd": payload.cost_usd,
        "input_tokens": payload.input_tokens,
        "output_tokens": payload.output_tokens,
        "exit_code": payload.exit_code,
        "summary": payload.summary,
    });

    let topic = format!(
        "lifecycle/codex/turn-complete/{}",
        payload.session_id.as_deref().unwrap_or("unknown")
    );

    // Note: store_in_hyphae is fire-and-forget with internal error logging.
    // Failures are logged as warn! inside hyphae_client::store_in_hyphae().
    store_in_hyphae(
        &topic,
        &signal.to_string(),
        Importance::Medium,
        project.as_deref(),
        None,
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_codex_turn_payload() {
        let input = r#"{
            "session_id": "codex-test-123",
            "turn_number": 1,
            "model": "codex-4",
            "cwd": "/tmp/test",
            "cost_usd": 0.012,
            "input_tokens": 1200,
            "output_tokens": 340,
            "exit_code": 0,
            "summary": "Test turn"
        }"#;

        let result = handle_turn_complete(input);
        assert!(result.is_ok(), "should handle full payload");
    }

    #[test]
    fn handles_empty_json() {
        let input = "{}";
        let result = handle_turn_complete(input);
        assert!(result.is_ok(), "should handle empty object");
    }

    #[test]
    fn handles_malformed_json() {
        let input = "{not-json}";
        let result = handle_turn_complete(input);
        assert!(result.is_ok(), "should handle malformed json gracefully");
    }
}

use std::fs;
use std::path::PathBuf;

use crate::events::OutcomeEvent;
use crate::utils::{load_json_file, save_json_file, temp_state_path};

const MAX_OUTCOME_EVENTS: usize = 128;

fn outcomes_path(hash: &str) -> PathBuf {
    temp_state_path("outcomes", hash, "json")
}

pub fn load_outcomes(hash: &str) -> Vec<OutcomeEvent> {
    load_json_file(outcomes_path(hash)).unwrap_or_default()
}

pub fn record_outcome(hash: &str, event: OutcomeEvent) {
    let mut events = load_outcomes(hash);
    events.push(event);

    if events.len() > MAX_OUTCOME_EVENTS {
        let overflow = events.len().saturating_sub(MAX_OUTCOME_EVENTS);
        events.drain(0..overflow);
    }

    let _ = save_json_file(outcomes_path(hash), &events);
}

pub fn clear_outcomes(hash: &str) {
    let _ = fs::remove_file(outcomes_path(hash));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{OutcomeEvent, OutcomeKind};

    fn test_hash(name: &str) -> String {
        format!("test-{}-{name}", std::process::id())
    }

    #[test]
    fn records_and_loads_outcomes() {
        let hash = test_hash("roundtrip");
        clear_outcomes(&hash);

        record_outcome(
            &hash,
            OutcomeEvent::new(OutcomeKind::ValidationPassed, "cargo test passed")
                .with_command("cargo test")
                .with_signal_type("test_passed"),
        );

        let outcomes = load_outcomes(&hash);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].kind, OutcomeKind::ValidationPassed);
        assert_eq!(outcomes[0].command.as_deref(), Some("cargo test"));

        clear_outcomes(&hash);
    }

    #[test]
    fn trims_oldest_outcomes_when_limit_is_exceeded() {
        let hash = test_hash("trim");
        clear_outcomes(&hash);

        for idx in 0..(MAX_OUTCOME_EVENTS + 5) {
            record_outcome(
                &hash,
                OutcomeEvent::new(OutcomeKind::ErrorDetected, format!("failure {idx}")),
            );
        }

        let outcomes = load_outcomes(&hash);
        assert_eq!(outcomes.len(), MAX_OUTCOME_EVENTS);
        assert_eq!(
            outcomes.first().map(|event| event.summary.as_str()),
            Some("failure 5")
        );

        clear_outcomes(&hash);
    }
}

use std::path::PathBuf;

use crate::events::OutcomeEvent;
use crate::utils::{load_json_file, remove_file_with_lock, temp_state_path, update_json_file};

const MAX_OUTCOME_EVENTS: usize = 128;

fn outcomes_path(hash: &str) -> PathBuf {
    temp_state_path("outcomes", hash, "json")
}

pub fn load_outcomes(hash: &str) -> Vec<OutcomeEvent> {
    load_json_file(outcomes_path(hash)).unwrap_or_default()
}

pub fn record_outcome(hash: &str, event: OutcomeEvent) {
    let _ = update_json_file::<Vec<OutcomeEvent>, _, _>(outcomes_path(hash), |events| {
        events.push(event);

        if events.len() > MAX_OUTCOME_EVENTS {
            let overflow = events.len().saturating_sub(MAX_OUTCOME_EVENTS);
            events.drain(0..overflow);
        }
    });
}

pub fn clear_outcomes(hash: &str) {
    let _ = remove_file_with_lock(outcomes_path(hash));
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

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

    #[test]
    fn preserves_concurrent_outcome_writes() {
        let hash = test_hash("concurrent");
        clear_outcomes(&hash);

        let writers = 12;
        let barrier = Arc::new(Barrier::new(writers));
        let mut handles = Vec::new();

        for idx in 0..writers {
            let hash = hash.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                record_outcome(
                    &hash,
                    OutcomeEvent::new(OutcomeKind::ValidationPassed, format!("writer-{idx}")),
                );
            }));
        }

        for handle in handles {
            handle.join().expect("thread should finish cleanly");
        }

        let outcomes = load_outcomes(&hash);
        assert_eq!(outcomes.len(), writers);
        for idx in 0..writers {
            assert!(
                outcomes
                    .iter()
                    .any(|event| event.summary == format!("writer-{idx}")),
                "missing outcome from writer-{idx}"
            );
        }

        clear_outcomes(&hash);
    }
}

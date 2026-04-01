use std::path::PathBuf;

use crate::events::OutcomeEvent;
use crate::policy::capture_policy;
use crate::utils::{load_json_file, remove_file_with_lock, temp_state_path, update_json_file};

fn outcomes_path(hash: &str) -> PathBuf {
    temp_state_path("outcomes", hash, "json")
}

pub fn load_outcomes(hash: &str) -> Vec<OutcomeEvent> {
    load_json_file(outcomes_path(hash)).unwrap_or_default()
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "The owned event is cloned into storage and may also be attached as evidence"
)]
pub fn record_outcome(hash: &str, event: OutcomeEvent) -> bool {
    let policy = capture_policy().clone();
    let event_for_storage = event.clone();
    let inserted =
        update_json_file::<Vec<OutcomeEvent>, _, _>(outcomes_path(hash), move |events| {
            let is_duplicate = events.iter().rev().any(|existing| {
                existing.semantically_matches(&event_for_storage)
                    && event_for_storage
                        .timestamp
                        .saturating_sub(existing.timestamp)
                        <= policy.outcome_dedupe_window_ms
            });

            if is_duplicate {
                return false;
            }

            events.push(event_for_storage.clone());

            if events.len() > policy.max_outcome_events {
                let overflow = events.len().saturating_sub(policy.max_outcome_events);
                events.drain(0..overflow);
            }

            true
        })
        .unwrap_or(false);

    #[cfg(not(test))]
    if inserted {
        crate::utils::attach_outcome_evidence(&event);
    }

    inserted
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

        assert!(record_outcome(
            &hash,
            OutcomeEvent::new(OutcomeKind::ValidationPassed, "cargo test passed")
                .with_command("cargo test")
                .with_signal_type("test_passed"),
        ));

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
        let max_events = crate::policy::capture_policy().max_outcome_events;

        for idx in 0..(max_events + 5) {
            assert!(record_outcome(
                &hash,
                OutcomeEvent::new(OutcomeKind::ErrorDetected, format!("failure {idx}")),
            ));
        }

        let outcomes = load_outcomes(&hash);
        assert_eq!(
            outcomes.len(),
            crate::policy::capture_policy().max_outcome_events
        );
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
                assert!(record_outcome(
                    &hash,
                    OutcomeEvent::new(OutcomeKind::ValidationPassed, format!("writer-{idx}")),
                ));
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

    #[test]
    fn suppresses_duplicate_outcomes_within_policy_window() {
        let hash = test_hash("dedupe");
        clear_outcomes(&hash);

        let event = OutcomeEvent::new(OutcomeKind::ValidationPassed, "cargo test passed")
            .with_command("cargo test")
            .with_signal_type("test_passed");

        assert!(record_outcome(&hash, event.clone()));
        assert!(!record_outcome(&hash, event));

        let outcomes = load_outcomes(&hash);
        assert_eq!(outcomes.len(), 1);

        clear_outcomes(&hash);
    }
}

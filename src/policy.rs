use std::env;
use std::sync::OnceLock;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct CapturePolicy {
    pub outcome_dedupe_window_ms: u64,
    pub correction_window_ms: u64,
    pub edit_cleanup_age_ms: u64,
    pub export_threshold: usize,
    pub ingest_threshold: usize,
    pub rhizome_suggest_threshold: usize,
    pub rhizome_suggest_enabled: bool,
    pub outcome_attribution_grace_ms: u64,
    pub max_outcome_events: usize,
    pub fallback_session_memory_on_end_failure: bool,
}

pub fn capture_policy() -> &'static CapturePolicy {
    static POLICY: OnceLock<CapturePolicy> = OnceLock::new();
    POLICY.get_or_init(CapturePolicy::from_env)
}

impl CapturePolicy {
    fn from_env() -> Self {
        Self::from_reader(|name| env::var(name).ok())
    }

    fn from_reader(read_env: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            outcome_dedupe_window_ms: read_u64(
                &read_env,
                "CORTINA_OUTCOME_DEDUPE_WINDOW_MS",
                30_000,
            ),
            correction_window_ms: read_u64(
                &read_env,
                "CORTINA_CORRECTION_WINDOW_MS",
                5 * 60 * 1000,
            ),
            edit_cleanup_age_ms: read_u64(&read_env, "CORTINA_EDIT_CLEANUP_AGE_MS", 10 * 60 * 1000),
            export_threshold: read_usize(&read_env, "CORTINA_EXPORT_THRESHOLD", 5),
            ingest_threshold: read_usize(&read_env, "CORTINA_INGEST_THRESHOLD", 3),
            rhizome_suggest_threshold: read_usize(
                &read_env,
                "CORTINA_RHIZOME_SUGGEST_THRESHOLD",
                100,
            ),
            rhizome_suggest_enabled: read_bool(&read_env, "CORTINA_RHIZOME_SUGGEST_ENABLED", true),
            outcome_attribution_grace_ms: read_u64(
                &read_env,
                "CORTINA_OUTCOME_ATTRIBUTION_GRACE_MS",
                30_000,
            ),
            max_outcome_events: read_usize(&read_env, "CORTINA_MAX_OUTCOME_EVENTS", 128),
            fallback_session_memory_on_end_failure: read_bool(
                &read_env,
                "CORTINA_FALLBACK_SESSION_MEMORY_ON_END_FAILURE",
                false,
            ),
        }
    }
}

fn read_u64(read_env: &impl Fn(&str) -> Option<String>, name: &str, default: u64) -> u64 {
    read_env(name)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn read_usize(read_env: &impl Fn(&str) -> Option<String>, name: &str, default: usize) -> usize {
    read_env(name)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn read_bool(read_env: &impl Fn(&str) -> Option<String>, name: &str, default: bool) -> bool {
    read_env(name).map_or(default, |value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::CapturePolicy;

    #[test]
    fn reads_policy_overrides_from_env_reader() {
        let policy = CapturePolicy::from_reader(|name| match name {
            "CORTINA_OUTCOME_DEDUPE_WINDOW_MS" => Some("45000".to_string()),
            "CORTINA_CORRECTION_WINDOW_MS" => Some("1234".to_string()),
            "CORTINA_EDIT_CLEANUP_AGE_MS" => Some("5678".to_string()),
            "CORTINA_EXPORT_THRESHOLD" => Some("9".to_string()),
            "CORTINA_INGEST_THRESHOLD" => Some("7".to_string()),
            "CORTINA_RHIZOME_SUGGEST_THRESHOLD" => Some("250".to_string()),
            "CORTINA_RHIZOME_SUGGEST_ENABLED" => Some("false".to_string()),
            "CORTINA_OUTCOME_ATTRIBUTION_GRACE_MS" => Some("60000".to_string()),
            "CORTINA_MAX_OUTCOME_EVENTS" => Some("55".to_string()),
            "CORTINA_FALLBACK_SESSION_MEMORY_ON_END_FAILURE" => Some("true".to_string()),
            _ => None,
        });

        assert_eq!(policy.outcome_dedupe_window_ms, 45_000);
        assert_eq!(policy.correction_window_ms, 1_234);
        assert_eq!(policy.edit_cleanup_age_ms, 5_678);
        assert_eq!(policy.export_threshold, 9);
        assert_eq!(policy.ingest_threshold, 7);
        assert_eq!(policy.rhizome_suggest_threshold, 250);
        assert!(!policy.rhizome_suggest_enabled);
        assert_eq!(policy.outcome_attribution_grace_ms, 60_000);
        assert_eq!(policy.max_outcome_events, 55);
        assert!(policy.fallback_session_memory_on_end_failure);
    }

    #[test]
    fn falls_back_to_defaults_for_invalid_values() {
        let policy = CapturePolicy::from_reader(|name| match name {
            "CORTINA_OUTCOME_DEDUPE_WINDOW_MS" => Some("bad".to_string()),
            "CORTINA_EXPORT_THRESHOLD" => Some("nope".to_string()),
            "CORTINA_FALLBACK_SESSION_MEMORY_ON_END_FAILURE" => Some("off".to_string()),
            "CORTINA_RHIZOME_SUGGEST_THRESHOLD" => Some("bad".to_string()),
            _ => None,
        });

        assert_eq!(policy.outcome_dedupe_window_ms, 30_000);
        assert_eq!(policy.export_threshold, 5);
        assert_eq!(policy.rhizome_suggest_threshold, 100);
        assert!(policy.rhizome_suggest_enabled);
        assert!(!policy.fallback_session_memory_on_end_failure);
    }
}

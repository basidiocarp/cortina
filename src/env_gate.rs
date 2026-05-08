use crate::adapters::ClaudeCodeEventCommand;

/// Hook profile gating: determine if an event should be skipped based on env vars.
///
/// Supports two environment variables:
/// - `CORTINA_HOOK_PROFILE`: control which event types are allowed globally
///   - `minimal`: only `Stop` and `SessionEnd` events fire
///   - `standard` (or unset): all events fire (default behavior)
///   - `strict`: all events fire with extra validation (reserved for future use)
/// - `CORTINA_DISABLED_HOOKS`: comma-separated list of `PascalCase` event type names to suppress.
///   Examples: `PostToolUse`, `PreCompact`, `PostToolUse,PreCompact`.
///   Takes precedence over the profile.
///   Note: `policy.rs` also reads `CORTINA_DISABLED_HOOKS` but uses `snake_case` internal hook names
///   (e.g., `post_tool_use`). The two mechanisms coexist — they read the same env var but match
///   against different value formats and operate at different layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvGate {
    pub profile: HookProfile,
    pub disabled_events: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookProfile {
    Minimal,
    Standard,
    Strict,
}

impl EnvGate {
    pub fn from_env() -> Self {
        Self::from_reader(|name| std::env::var(name).ok())
    }

    pub fn from_reader(read_env: impl Fn(&str) -> Option<String>) -> Self {
        let profile = read_profile(&read_env);
        let disabled_events = read_disabled_events(&read_env, "CORTINA_DISABLED_HOOKS");
        Self {
            profile,
            disabled_events,
        }
    }

    /// Returns true if the event should be skipped based on profile and disabled list.
    pub fn should_skip_event(&self, event: ClaudeCodeEventCommand) -> bool {
        let event_name = event_to_name(event);

        // Check CORTINA_DISABLED_HOOKS first (highest priority, PascalCase event type names)
        if self
            .disabled_events
            .iter()
            .any(|e| e.eq_ignore_ascii_case(event_name))
        {
            return true;
        }

        // Check profile restrictions
        match self.profile {
            HookProfile::Minimal => {
                // Only Stop and SessionEnd are allowed in minimal
                !matches!(
                    event,
                    ClaudeCodeEventCommand::Stop | ClaudeCodeEventCommand::SessionEnd
                )
            }
            HookProfile::Standard | HookProfile::Strict => false,
        }
    }
}

fn read_profile(read_env: &impl Fn(&str) -> Option<String>) -> HookProfile {
    match read_env("CORTINA_HOOK_PROFILE")
        .map(|s| s.to_lowercase())
        .as_deref()
    {
        Some("minimal") => HookProfile::Minimal,
        Some("strict") => HookProfile::Strict,
        Some("standard" | "" | _) | None => HookProfile::Standard,
    }
}

fn read_disabled_events(read_env: &impl Fn(&str) -> Option<String>, name: &str) -> Vec<String> {
    read_env(name)
        .map(|val| {
            val.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn event_to_name(event: ClaudeCodeEventCommand) -> &'static str {
    match event {
        ClaudeCodeEventCommand::PreToolUse => "PreToolUse",
        ClaudeCodeEventCommand::PostToolUse => "PostToolUse",
        ClaudeCodeEventCommand::UserPromptSubmit => "UserPromptSubmit",
        ClaudeCodeEventCommand::PreCompact => "PreCompact",
        ClaudeCodeEventCommand::Stop => "Stop",
        ClaudeCodeEventCommand::SessionEnd => "SessionEnd",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a gate with specific overrides without touching the global env
    fn gate_with_profile_and_disabled(profile: &str, disabled: &[&str]) -> EnvGate {
        EnvGate::from_reader(|name| match name {
            "CORTINA_HOOK_PROFILE" => {
                if profile.is_empty() {
                    None
                } else {
                    Some(profile.to_string())
                }
            }
            "CORTINA_DISABLED_HOOKS" => {
                if disabled.is_empty() {
                    None
                } else {
                    Some(disabled.join(","))
                }
            }
            _ => None,
        })
    }

    #[test]
    fn minimal_profile_skips_tool_use_events() {
        let gate = gate_with_profile_and_disabled("minimal", &[]);

        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PreToolUse));
        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PostToolUse));
        assert!(gate.should_skip_event(ClaudeCodeEventCommand::UserPromptSubmit));
        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PreCompact));
    }

    #[test]
    fn minimal_profile_allows_stop_and_session_end() {
        let gate = gate_with_profile_and_disabled("minimal", &[]);

        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::Stop));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::SessionEnd));
    }

    #[test]
    fn standard_profile_allows_all_events() {
        let gate = gate_with_profile_and_disabled("standard", &[]);

        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::PreToolUse));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::PostToolUse));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::UserPromptSubmit));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::PreCompact));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::Stop));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::SessionEnd));
    }

    #[test]
    fn unset_profile_defaults_to_standard() {
        let gate = gate_with_profile_and_disabled("", &[]);

        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::PreToolUse));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::Stop));
    }

    #[test]
    fn unknown_profile_defaults_to_standard() {
        let gate = gate_with_profile_and_disabled("unknown_profile", &[]);

        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::PreToolUse));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::Stop));
    }

    #[test]
    fn disabled_events_list_skips_specific_events() {
        let gate = gate_with_profile_and_disabled("standard", &["PostToolUse", "PreCompact"]);

        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::PreToolUse));
        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PostToolUse));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::UserPromptSubmit));
        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PreCompact));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::Stop));
    }

    #[test]
    fn disabled_events_is_case_insensitive() {
        let gate = gate_with_profile_and_disabled("standard", &["posttooluse", "PRECOMPACT"]);

        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PostToolUse));
        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PreCompact));
    }

    #[test]
    fn disabled_events_takes_precedence_over_profile() {
        // Even in strict profile, a disabled event should still be skipped
        let gate = gate_with_profile_and_disabled("strict", &["PostToolUse"]);

        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PostToolUse));
        assert!(!gate.should_skip_event(ClaudeCodeEventCommand::PreToolUse));
    }

    #[test]
    fn disabled_events_with_whitespace_is_trimmed() {
        let gate = EnvGate::from_reader(|name| match name {
            "CORTINA_DISABLED_HOOKS" => Some("  PostToolUse  ,  PreCompact  ".to_string()),
            _ => None,
        });

        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PostToolUse));
        assert!(gate.should_skip_event(ClaudeCodeEventCommand::PreCompact));
    }

    #[test]
    fn event_to_name_returns_correct_strings() {
        assert_eq!(
            event_to_name(ClaudeCodeEventCommand::PreToolUse),
            "PreToolUse"
        );
        assert_eq!(
            event_to_name(ClaudeCodeEventCommand::PostToolUse),
            "PostToolUse"
        );
        assert_eq!(
            event_to_name(ClaudeCodeEventCommand::UserPromptSubmit),
            "UserPromptSubmit"
        );
        assert_eq!(
            event_to_name(ClaudeCodeEventCommand::PreCompact),
            "PreCompact"
        );
        assert_eq!(event_to_name(ClaudeCodeEventCommand::Stop), "Stop");
        assert_eq!(
            event_to_name(ClaudeCodeEventCommand::SessionEnd),
            "SessionEnd"
        );
    }
}

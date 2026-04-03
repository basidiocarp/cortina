use anyhow::{Context, Result, ensure};

use crate::events::VolvaHookEvent;
#[cfg(test)]
use crate::utils::{load_json_file, remove_file_with_lock};
use crate::utils::{scope_hash, temp_state_path, update_json_file};

const MAX_RECORDED_VOLVA_HOOK_EVENTS: usize = 32;

pub fn handle_hook_event(input: &str) -> Result<()> {
    let event = parse_hook_event(input)?;
    record_hook_event(&event)
}

fn parse_hook_event(input: &str) -> Result<VolvaHookEvent> {
    let event: VolvaHookEvent =
        serde_json::from_str(input).context("failed to parse volva hook event JSON")?;
    validate_hook_event(&event)?;
    Ok(event)
}

fn validate_hook_event(event: &VolvaHookEvent) -> Result<()> {
    ensure!(
        !event.cwd.trim().is_empty(),
        "volva hook event `cwd` must not be empty"
    );
    ensure!(
        !(event.prompt_summary.trim().is_empty() && event.prompt_text.trim().is_empty()),
        "volva hook event must include `prompt_summary` or `prompt_text`"
    );
    Ok(())
}

fn record_hook_event(event: &VolvaHookEvent) -> Result<()> {
    let hash = scope_hash(Some(&event.cwd));
    let event = event.clone();
    update_json_file::<Vec<VolvaHookEvent>, _, _>(volva_hook_events_path(&hash), move |events| {
        events.push(event);

        if events.len() > MAX_RECORDED_VOLVA_HOOK_EVENTS {
            let overflow = events.len().saturating_sub(MAX_RECORDED_VOLVA_HOOK_EVENTS);
            events.drain(0..overflow);
        }
    })?;
    Ok(())
}

fn volva_hook_events_path(hash: &str) -> std::path::PathBuf {
    temp_state_path("volva-hook-events", hash, "json")
}

#[cfg(test)]
fn load_recorded_hook_events(cwd: &str) -> Vec<VolvaHookEvent> {
    let hash = scope_hash(Some(cwd));
    load_json_file(volva_hook_events_path(&hash)).unwrap_or_default()
}

#[cfg(test)]
fn clear_recorded_hook_events(cwd: &str) {
    let hash = scope_hash(Some(cwd));
    let _ = remove_file_with_lock(volva_hook_events_path(&hash));
}

#[cfg(test)]
mod tests {
    use std::env;

    use serde_json::json;

    use super::{clear_recorded_hook_events, handle_hook_event, load_recorded_hook_events};
    use crate::events::{VolvaBackendKind, VolvaHookPhase};

    #[test]
    fn parses_and_records_volva_hook_event() {
        let cwd = unique_cwd("success");
        clear_recorded_hook_events(&cwd);

        let input = json!({
            "phase": "before_prompt_send",
            "backend_kind": "official-cli",
            "cwd": cwd,
            "prompt_text": "summarize the repository",
            "prompt_summary": "summarize the repository",
            "stdout": "assistant output",
            "stderr": "diagnostic text",
            "exit_code": 0
        })
        .to_string();

        handle_hook_event(&input).expect("volva hook event should be accepted");

        let events = load_recorded_hook_events(&cwd);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].phase, VolvaHookPhase::BeforePromptSend);
        assert_eq!(events[0].backend_kind, VolvaBackendKind::OfficialCli);
        assert_eq!(events[0].stdout.as_deref(), Some("assistant output"));
        assert_eq!(events[0].stderr.as_deref(), Some("diagnostic text"));
        assert_eq!(events[0].exit_code, Some(0));

        clear_recorded_hook_events(&cwd);
    }

    #[test]
    fn rejects_malformed_json() {
        let error =
            handle_hook_event("{not-json").expect_err("malformed json should return an error");

        assert!(
            error
                .to_string()
                .contains("failed to parse volva hook event JSON"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_unknown_phase() {
        let input = json!({
            "phase": "tool_intercepted",
            "backend_kind": "official-cli",
            "cwd": unique_cwd("unknown-phase"),
            "prompt_text": "x",
            "prompt_summary": "x"
        })
        .to_string();

        let error = handle_hook_event(&input).expect_err("unknown phase should be rejected");
        assert!(
            error
                .to_string()
                .contains("failed to parse volva hook event JSON"),
            "unexpected error: {error}"
        );
    }

    fn unique_cwd(name: &str) -> String {
        let stamp = crate::utils::current_timestamp_ms();
        env::temp_dir()
            .join(format!("cortina-volva-hook-{stamp}-{name}"))
            .to_string_lossy()
            .to_string()
    }
}

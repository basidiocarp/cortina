mod summary;
#[cfg(test)]
mod tests;
mod transcript;

use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::outcomes::{clear_outcomes, load_outcomes};
use crate::utils::{
    command_exists, end_scoped_hyphae_session, load_session_state,
    log_hyphae_feedback_signal_for_session, project_name_for_cwd, scope_hash,
    session_outcome_feedback,
};

use self::summary::{
    filter_outcomes_for_session, format_structured_outcome_attribution, merge_structured_outcomes,
};
use self::transcript::parse_transcript;

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> Result<()> {
    let envelope = match ClaudeCodeHookEnvelope::parse(input) {
        Ok(envelope) => envelope,
        Err(e) => {
            eprintln!("cortina: failed to parse event input: {e}");
            return Ok(());
        }
    };

    let Some(event) = envelope.session_stop_event() else {
        return Ok(());
    };

    if event.cwd.is_empty() {
        return Ok(());
    }

    let hash = scope_hash(Some(&event.cwd));
    let cached_session = load_session_state(&hash);
    let had_cached_session = cached_session.is_some();

    if !command_exists("hyphae") {
        clear_outcomes(&hash);
        return Ok(());
    }

    let project_name = project_name_for_cwd(Some(&event.cwd)).unwrap_or_else(|| {
        Path::new(&event.cwd)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    });
    let structured_outcomes = filter_outcomes_for_session(
        &load_outcomes(&hash),
        cached_session.as_ref(),
        &project_name,
    );

    let summary = merge_structured_outcomes(
        parse_transcript(event.transcript_path.as_deref()),
        &structured_outcomes,
    );

    let mut text = format!("Session in {project_name}: {}", summary.task_desc);

    if !summary.files_modified.is_empty() {
        let _ = write!(text, "\nFiles: {}", summary.files_modified.join(", "));
    }

    if !summary.tool_counts.is_empty() {
        let _ = write!(text, "\nTools: {}", summary.tool_counts);
    }

    if summary.errors_encountered > 0 {
        let _ = write!(text, "\nErrors encountered: {}", summary.errors_encountered);
    }

    if !summary.outcome.is_empty() {
        let _ = write!(text, "\nOutcome: {}", summary.outcome);
    }

    if let Some(attribution) = format_structured_outcome_attribution(&structured_outcomes) {
        let _ = write!(text, "\nStructured outcomes: {attribution}");
    }

    let session_feedback = session_outcome_feedback(
        &summary.outcome,
        summary::has_unresolved_errors(&structured_outcomes),
    );

    let ended_structured_session = end_scoped_hyphae_session(
        Some(&event.cwd),
        Some(&text),
        &summary.files_modified,
        summary.errors_encountered,
    );

    if let Some(ref state) = ended_structured_session {
        log_hyphae_feedback_signal_for_session(
            state,
            session_feedback.0,
            session_feedback.1,
            session_feedback.2,
        );
        clear_outcomes(&hash);
    } else if !had_cached_session {
        clear_outcomes(&hash);
    }

    Ok(())
}

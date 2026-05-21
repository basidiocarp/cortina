use super::parse_error::parse_or_allow;
use crate::events::PostCompactEvent;

#[allow(
    clippy::unnecessary_wraps,
    reason = "Result return type required by dispatch match in main"
)]
pub fn handle(input: &str) -> anyhow::Result<()> {
    let Some(envelope) = parse_or_allow(input) else {
        return Ok(());
    };

    let Some(event) = envelope.post_compact_event() else {
        return Ok(());
    };

    capture_post_compact(&event);
    Ok(())
}

fn capture_post_compact(event: &PostCompactEvent) {
    tracing::info!(
        session_id = %event.session_id,
        cwd = %event.cwd,
        transcript_path = ?event.transcript_path,
        "cortina: post_compact event received"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_returns_ok_for_valid_input() {
        // Even with invalid JSON, parse_or_allow returns None and we continue gracefully
        let result = handle("{}");
        assert!(result.is_ok());
    }

    #[test]
    fn handle_returns_ok_for_empty_input() {
        let result = handle("");
        assert!(result.is_ok());
    }
}

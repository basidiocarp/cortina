#[cfg(not(test))]
use std::io::Write;

/// Emit OSC 9 and OSC 777 notification sequences to stderr.
/// This allows OSC-aware terminals to surface agent-needs-attention signals.
///
/// OSC 9 format: `\x1b]9;<body>\x07`
/// OSC 777 format: `\x1b]777;notify;<title>;<body>\x07`
///
/// The actual stderr writes are guarded with `#[cfg(not(test))]` so tests remain silent.
pub fn emit_osc_notification(title: &str, body: &str) {
    // The `cortina notify --title/--body` CLI accepts arbitrary operator input.
    // Control characters (notably ESC `\x1b` and BEL `\x07`) would terminate the
    // OSC sequence early and let the caller inject arbitrary terminal escape
    // sequences; a `;` in the OSC 777 title field would split it into spurious
    // fields. Sanitize at this boundary so neither the hook nor the CLI can emit
    // attacker-controlled terminal control sequences.
    let title = sanitize(title, true);
    let body = sanitize(body, false);
    let osc_9 = format_osc_9(&body);
    let osc_777 = format_osc_777(&title, &body);

    #[cfg(not(test))]
    {
        let mut stderr = std::io::stderr();
        // Swallow write errors — this is fire-and-forget notification.
        let _ = stderr.write_all(osc_9.as_bytes());
        let _ = stderr.write_all(osc_777.as_bytes());
    }

    // Tests use the format fns to verify the sequences without writing to stderr.
    #[cfg(test)]
    {
        // Prevent unused variable warnings in test builds.
        let _ = (osc_9, osc_777);
    }
}

/// Strip bytes that could break out of or corrupt an OSC sequence.
///
/// Control characters (including ESC `\x1b` and BEL `\x07`) are always removed:
/// they would terminate the sequence early or inject further escape sequences.
/// When `strip_field_separator` is set (the OSC 777 title field), `;` is also
/// removed so the caller cannot split the field into spurious sub-fields.
fn sanitize(s: &str, strip_field_separator: bool) -> String {
    s.chars()
        .filter(|&c| {
            if c.is_control() {
                return false;
            }
            if strip_field_separator && c == ';' {
                return false;
            }
            true
        })
        .collect()
}

/// Format an OSC 9 notification sequence.
fn format_osc_9(body: &str) -> String {
    format!("\x1b]9;{body}\x07")
}

/// Format an OSC 777 notify sequence.
fn format_osc_777(title: &str, body: &str) -> String {
    format!("\x1b]777;notify;{title};{body}\x07")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_9_format_is_correct() {
        let body = "test project";
        let result = format_osc_9(body);
        assert_eq!(result, "\x1b]9;test project\x07");
    }

    #[test]
    fn osc_777_format_is_correct() {
        let title = "cortina: session captured";
        let body = "test project";
        let result = format_osc_777(title, body);
        assert_eq!(
            result,
            "\x1b]777;notify;cortina: session captured;test project\x07"
        );
    }

    #[test]
    fn osc_777_passes_through_body_verbatim() {
        // Slashes and other printable, non-control characters are passed through
        // unchanged; only control chars and (in the title) `;` are sanitized.
        let title = "cortina: session captured";
        let body = "project/with/slashes";
        let osc_777 = format_osc_777(title, body);
        assert!(osc_777.contains("project/with/slashes"));
        assert!(osc_777.starts_with("\x1b]777;notify;"));
        assert!(osc_777.ends_with('\x07'));
    }

    #[test]
    fn sanitize_strips_control_chars() {
        // ESC and BEL would terminate/inject the OSC sequence; they must be removed.
        assert_eq!(sanitize("a\x1bb\x07c\n", false), "abc");
    }

    #[test]
    fn sanitize_title_strips_field_separator() {
        assert_eq!(sanitize("a;b;c", true), "abc");
    }

    #[test]
    fn sanitize_body_keeps_field_separator() {
        // `;` in the OSC 777 body (the final field) is harmless, so it is kept.
        assert_eq!(sanitize("a;b", false), "a;b");
    }

    #[test]
    fn sanitize_strips_c1_control_codepoints() {
        // C1 controls ST (U+009C) and CSI (U+009B) are single-codepoint
        // terminal-escape vectors a terminal treats as sequence breaks.
        // `char::is_control()` covers the C1 range (0x80–0x9F), so they are stripped.
        assert_eq!(sanitize("a\u{9c}\u{9b}b", false), "ab");
    }

    #[test]
    fn emit_osc_notification_does_not_panic() {
        // In test builds, emit_osc_notification should not write to stderr.
        // This just verifies it's infallible.
        emit_osc_notification("cortina: session captured", "test project");
    }
}

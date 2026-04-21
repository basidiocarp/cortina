use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolAvailableItem {
    tool_name: String,
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCalledItem {
    tool_name: String,
    source: String,
    call_count: u32,
    first_call_at: String,
    last_call_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolRelevantUnusedItem {
    tool_name: String,
    source: String,
    relevance_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolUsageEventV1 {
    schema_version: String,
    session_id: String,
    host: String,
    timestamp: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools_available: Vec<ToolAvailableItem>,
    tools_called: Vec<ToolCalledItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools_relevant_unused: Vec<ToolRelevantUnusedItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
}

/// Convert a Unix-epoch millisecond timestamp to an ISO 8601 string.
///
/// NOTE: The `is_leap_year` helper below uses the standard Gregorian rule
/// (divisible by 4, except centuries unless also divisible by 400). This is
/// correct through at least 2100 — the next century-year that *is not* a leap
/// year. Code that runs past 2100 will produce off-by-one day errors for dates
/// in leap-century years (2200, 2300, …). Replace this with `chrono` or
/// `humantime` when either becomes available as a project dependency.
fn ms_to_iso8601(ms: u64) -> String {
    const SECS_PER_DAY: u64 = 86_400;
    const SECS_PER_HOUR: u64 = 3_600;
    const SECS_PER_MINUTE: u64 = 60;

    let secs = ms / 1000;
    let subsec_millis = (ms % 1000) as u32;

    let days_since_epoch = secs / SECS_PER_DAY;
    let secs_today = secs % SECS_PER_DAY;

    let hours = secs_today / SECS_PER_HOUR;
    let remaining = secs_today % SECS_PER_HOUR;
    let minutes = remaining / SECS_PER_MINUTE;
    let seconds = remaining % SECS_PER_MINUTE;

    let mut year: i32 = 1970;
    let mut days_remaining: i32 =
        i32::try_from(days_since_epoch.min(i32::MAX as u64)).unwrap_or(i32::MAX);

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days_remaining < days_in_year {
            break;
        }
        days_remaining -= days_in_year;
        year += 1;
    }

    // Calculate month and day
    let is_leap = is_leap_year(year);
    let days_in_months = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    let mut day = days_remaining + 1;
    for &days_in_month in &days_in_months {
        if day <= days_in_month {
            break;
        }
        day -= days_in_month;
        month += 1;
    }

    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{subsec_millis:03}Z"
    )
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

pub(super) fn emit_tool_usage_event(
    session_id: &str,
    task_id: Option<&str>,
    tool_calls: &[crate::tool_usage::ToolCallEntry],
    gaps: &[(String, String, String)],
) {
    if tool_calls.is_empty() {
        return;
    }

    let now = crate::utils::current_timestamp_ms();
    let timestamp = ms_to_iso8601(now);

    let tools_called = tool_calls
        .iter()
        .map(|entry| ToolCalledItem {
            tool_name: entry.tool_name.clone(),
            source: serde_json::to_value(&entry.source)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "other".to_string()),
            call_count: entry.call_count,
            first_call_at: ms_to_iso8601(entry.first_call_at_ms),
            last_call_at: ms_to_iso8601(entry.last_call_at_ms),
        })
        .collect::<Vec<_>>();

    let tools_relevant_unused = gaps
        .iter()
        .map(|(tool_name, source, reason)| ToolRelevantUnusedItem {
            tool_name: tool_name.clone(),
            source: source.clone(),
            relevance_reason: reason.clone(),
        })
        .collect::<Vec<_>>();

    let event = ToolUsageEventV1 {
        schema_version: "1.0".to_string(),
        session_id: session_id.to_string(),
        host: "claude_code".to_string(),
        timestamp,
        tools_available: Vec::new(),
        tools_called,
        tools_relevant_unused,
        task_id: task_id.map(str::to_string),
    };

    if let Ok(json) = serde_json::to_string(&event) {
        eprintln!("cortina: tool-usage-event: {json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::stop::compute_tool_adoption_gaps;
    use crate::tool_usage::{ToolCallEntry, ToolSource};

    fn make_tool_call(tool_name: &str) -> ToolCallEntry {
        ToolCallEntry {
            tool_name: tool_name.to_string(),
            source: ToolSource::Other,
            call_count: 1,
            first_call_at_ms: 0,
            last_call_at_ms: 0,
        }
    }

    #[test]
    fn ms_to_iso8601_produces_valid_format() {
        let ms = 1_776_245_200_000u64;
        let iso = ms_to_iso8601(ms);

        // Should be ISO 8601 format
        assert!(iso.contains('T'), "should contain T separator");
        assert!(iso.ends_with('Z'), "should end with Z for UTC");
        assert!(iso.contains('-'), "should contain date separators");
    }

    #[test]
    fn ms_to_iso8601_known_values() {
        assert_eq!(ms_to_iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(ms_to_iso8601(946_684_800_000), "2000-01-01T00:00:00.000Z");
    }

    #[test]
    fn serialized_event_contains_expected_fields() {
        let event = ToolUsageEventV1 {
            schema_version: "1.0".to_string(),
            session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
            host: "claude_code".to_string(),
            timestamp: "2026-04-14T10:00:00Z".to_string(),
            tools_available: Vec::new(),
            tools_called: vec![ToolCalledItem {
                tool_name: "test_tool".to_string(),
                source: "other".to_string(),
                call_count: 2,
                first_call_at: "1970-01-01T00:00:01Z".to_string(),
                last_call_at: "1970-01-01T00:00:02Z".to_string(),
            }],
            tools_relevant_unused: Vec::new(),
            task_id: None,
        };

        let json = serde_json::to_string(&event).expect("should serialize");
        assert!(json.contains("\"schema_version\":\"1.0\""));
        assert!(json.contains("\"session_id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAV\""));
        assert!(json.contains("\"tools_called\""));
        assert!(json.contains("\"call_count\":2"));
    }

    #[test]
    fn gaps_populate_tools_relevant_unused_in_serialized_event() {
        let gaps = [(
            "mcp__rhizome__get_structure".to_string(),
            "rules".to_string(),
            "recommended for Write on src/lib.rs".to_string(),
        )];

        let tools_relevant_unused = gaps
            .iter()
            .map(|(tool_name, source, reason)| ToolRelevantUnusedItem {
                tool_name: tool_name.clone(),
                source: source.clone(),
                relevance_reason: reason.clone(),
            })
            .collect::<Vec<_>>();

        let event = ToolUsageEventV1 {
            schema_version: "1.0".to_string(),
            session_id: "ses_test".to_string(),
            host: "claude_code".to_string(),
            timestamp: "2026-04-16T00:00:00.000Z".to_string(),
            tools_available: Vec::new(),
            tools_called: vec![ToolCalledItem {
                tool_name: "Bash".to_string(),
                source: "other".to_string(),
                call_count: 1,
                first_call_at: "1970-01-01T00:00:00.000Z".to_string(),
                last_call_at: "1970-01-01T00:00:00.000Z".to_string(),
            }],
            tools_relevant_unused,
            task_id: None,
        };

        let json = serde_json::to_string(&event).expect("should serialize");
        assert!(json.contains("\"tools_relevant_unused\""));
        assert!(json.contains("\"mcp__rhizome__get_structure\""));
        assert!(json.contains("\"relevance_reason\""));
    }

    #[test]
    fn empty_gaps_omit_tools_relevant_unused_from_serialized_event() {
        let event = ToolUsageEventV1 {
            schema_version: "1.0".to_string(),
            session_id: "ses_clean".to_string(),
            host: "claude_code".to_string(),
            timestamp: "2026-04-16T00:00:00.000Z".to_string(),
            tools_available: Vec::new(),
            tools_called: vec![ToolCalledItem {
                tool_name: "Bash".to_string(),
                source: "other".to_string(),
                call_count: 1,
                first_call_at: "1970-01-01T00:00:00.000Z".to_string(),
                last_call_at: "1970-01-01T00:00:00.000Z".to_string(),
            }],
            tools_relevant_unused: Vec::new(),
            task_id: None,
        };

        let json = serde_json::to_string(&event).expect("should serialize");
        assert!(!json.contains("tools_relevant_unused"), "empty vec should be skipped");
    }

    #[test]
    fn gap_detection_returns_gaps_when_no_rhizome_calls_on_rs_files() {
        let files = vec!["src/lib.rs".to_string()];
        let tool_calls = vec![
            make_tool_call("Bash"),
            make_tool_call("Read"),
        ];

        let gaps = compute_tool_adoption_gaps(&files, &tool_calls);
        assert!(!gaps.is_empty(), "expected gaps for .rs file without rhizome tools");

        let tool_names: Vec<&str> = gaps.iter().map(|(name, _, _)| name.as_str()).collect();
        assert!(
            tool_names.contains(&"mcp__rhizome__get_structure"),
            "should flag mcp__rhizome__get_structure"
        );
    }

    #[test]
    fn gap_detection_returns_no_gaps_when_rhizome_called_on_rs_files() {
        let files = vec!["src/main.rs".to_string()];
        let tool_calls = vec![
            make_tool_call("mcp__rhizome__get_structure"),
            make_tool_call("Bash"),
        ];

        let gaps = compute_tool_adoption_gaps(&files, &tool_calls);
        assert!(gaps.is_empty(), "rhizome call satisfies the rule; no gaps expected");
    }

    #[test]
    fn gap_detection_returns_no_gaps_for_clean_session_with_no_files() {
        let files: Vec<String> = vec![];
        let tool_calls = vec![make_tool_call("Bash")];

        let gaps = compute_tool_adoption_gaps(&files, &tool_calls);
        assert!(gaps.is_empty(), "no files modified; no gaps expected");
    }

    #[test]
    fn gap_detection_deduplicates_same_tool_across_multiple_files() {
        let files = vec![
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
        ];
        let tool_calls = vec![make_tool_call("Bash")];

        let gaps = compute_tool_adoption_gaps(&files, &tool_calls);
        let get_structure_count = gaps
            .iter()
            .filter(|(name, _, _)| name == "mcp__rhizome__get_structure")
            .count();
        assert_eq!(
            get_structure_count,
            1,
            "same tool should appear only once even across multiple files"
        );
    }
}

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

fn ms_to_iso8601(ms: u64) -> String {
    let secs = ms / 1000;
    let subsec_millis = (ms % 1000) as u32;

    // Unix epoch is 1970-01-01T00:00:00Z
    // Calculate days since epoch
    const SECS_PER_DAY: u64 = 86_400;
    const SECS_PER_HOUR: u64 = 3_600;
    const SECS_PER_MINUTE: u64 = 60;

    let days_since_epoch = secs / SECS_PER_DAY;
    let secs_today = secs % SECS_PER_DAY;

    let hours = secs_today / SECS_PER_HOUR;
    let remaining = secs_today % SECS_PER_HOUR;
    let minutes = remaining / SECS_PER_MINUTE;
    let seconds = remaining % SECS_PER_MINUTE;

    // Convert days to year/month/day
    // Simplified calculation: approximate year from days
    let mut year: i32 = 1970;
    let mut days_remaining: i32 = days_since_epoch.min(i32::MAX as u64) as i32;

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
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hours, minutes, seconds, subsec_millis
    )
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

pub(super) fn emit_tool_usage_event(
    session_id: &str,
    task_id: Option<&str>,
    tool_calls: &[crate::tool_usage::ToolCallEntry],
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

    let event = ToolUsageEventV1 {
        schema_version: "1.0".to_string(),
        session_id: session_id.to_string(),
        host: "claude_code".to_string(),
        timestamp,
        tools_available: Vec::new(),
        tools_called,
        tools_relevant_unused: Vec::new(),
        task_id: task_id.map(|id| id.to_string()),
    };

    if let Ok(json) = serde_json::to_string(&event) {
        eprintln!("cortina: tool-usage-event: {json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_to_iso8601_produces_valid_format() {
        let ms = 1776245200000u64;
        let iso = ms_to_iso8601(ms);

        // Should be ISO 8601 format
        assert!(iso.contains("T"), "should contain T separator");
        assert!(iso.ends_with("Z"), "should end with Z for UTC");
        assert!(iso.contains('-'), "should contain date separators");
    }

    #[test]
    fn ms_to_iso8601_known_values() {
        assert_eq!(ms_to_iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(ms_to_iso8601(946684800000), "2000-01-01T00:00:00.000Z");
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
}

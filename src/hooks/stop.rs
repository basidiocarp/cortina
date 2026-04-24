mod summary;
#[cfg(test)]
mod tests;
mod tool_usage_emit;
mod transcript;

use anyhow::Result;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::adapters::claude_code::ClaudeCodeHookEnvelope;
use crate::handoff_lint::audit_handoff as lint_handoff;
use crate::handoff_paths::{ChecklistItem, extract_paths, extract_paths_from_text};
use crate::outcomes::{clear_outcomes, load_outcomes};
use crate::policy::capture_policy;
use crate::rules::{DEFAULT_RULES, any_recommended_called, matching_rules};
use crate::utils::{
    command_exists, end_scoped_hyphae_session, load_session_state,
    log_hyphae_feedback_signal_for_session, project_name_for_cwd, scope_hash,
    session_outcome_feedback,
};

use self::summary::{
    filter_outcomes_for_session, format_structured_outcome_attribution, merge_structured_outcomes,
};
use self::transcript::parse_transcript;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleHandoffWarning {
    pub handoff_file: PathBuf,
    pub overlapping_files: Vec<String>,
    pub unchecked_items: Vec<String>,
    pub suggestion: String,
}

#[allow(
    clippy::too_many_lines,
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
        crate::tool_usage::clear_tool_calls(&hash);
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

    if capture_policy().stale_handoff_detection_enabled {
        for warning in check_handoff_staleness(&summary.files_modified, Path::new(&event.cwd)) {
            eprintln!(
                "cortina: stale handoff warning for {}: {}",
                warning.handoff_file.display(),
                warning.suggestion
            );
            eprintln!(
                "cortina: overlapping files: {}",
                warning.overlapping_files.join(", ")
            );
            if !warning.unchecked_items.is_empty() {
                eprintln!(
                    "cortina: unchecked items: {}",
                    warning.unchecked_items.join(" | ")
                );
            }
        }
    }

    if capture_policy().handoff_lint_enabled {
        let session_paths = summary
            .files_modified
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        for warning in check_handoff_completion(&session_paths, Path::new(&event.cwd)) {
            eprintln!("{warning}");
        }
    }

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

        // Emit tool usage event
        let tool_calls = crate::tool_usage::load_tool_calls(&hash);
        if !tool_calls.is_empty() {
            let gaps = compute_tool_adoption_gaps(&summary.files_modified, &tool_calls);
            if !gaps.is_empty() {
                eprintln!("cortina: tool adoption gaps detected:");
                for (tool_name, _source, reason) in &gaps {
                    eprintln!("  - {tool_name} ({reason})");
                }
            }
            tool_usage_emit::emit_tool_usage_event(&state.session_id, None, &tool_calls, &gaps);
        }
        crate::tool_usage::clear_tool_calls(&hash);
    } else if !had_cached_session {
        clear_outcomes(&hash);
        crate::tool_usage::clear_tool_calls(&hash);
    }

    // Run FP marker check processor
    let transcript_text = read_transcript_text(event.transcript_path.as_deref());
    if let Some(fp_summary) = crate::hooks::fp_check::FpCheckProcessor::process(&transcript_text) {
        tracing::info!("cortina stop: {fp_summary}");
    }

    // Run trigger word processor
    let final_message = extract_final_message(&transcript_text);
    let tw_config = crate::hooks::trigger_word::TriggerWordConfig::default();
    let tw_processor = crate::hooks::trigger_word::TriggerWordProcessor::new(tw_config);
    let payloads = tw_processor.scan(&final_message);
    if !payloads.is_empty() {
        crate::hooks::trigger_word::TriggerWordProcessor::store(&payloads);
    }

    Ok(())
}

/// Computes tool adoption gaps by checking which recommended tools were not called
/// for the files modified this session.
///
/// Returns a list of `(tool_name, source, reason)` triples, deduplicated by `tool_name`.
pub(super) fn compute_tool_adoption_gaps(
    files_modified: &[String],
    tool_calls: &[crate::tool_usage::ToolCallEntry],
) -> Vec<(String, String, String)> {
    let called_names: Vec<&str> = tool_calls.iter().map(|e| e.tool_name.as_str()).collect();

    let mut gaps: Vec<(String, String, String)> = Vec::new();

    for file in files_modified {
        for operation in &["Write", "Edit"] {
            for rule in matching_rules(DEFAULT_RULES, operation, Some(file.as_str())) {
                if !any_recommended_called(rule, &called_names) {
                    for &recommended in rule.recommended_tools {
                        if !gaps.iter().any(|(name, _, _)| name == recommended) {
                            gaps.push((
                                recommended.to_string(),
                                "rules".to_string(),
                                format!("recommended for {operation} on {file}"),
                            ));
                        }
                    }
                }
            }
        }
    }

    gaps
}

fn check_handoff_staleness(
    session_modified_files: &[String],
    cwd: &Path,
) -> Vec<StaleHandoffWarning> {
    let Some(workspace_root) = workspace_root_from_cwd(cwd) else {
        return Vec::new();
    };
    let ready_handoffs = ready_handoff_files(&workspace_root);
    let session_paths: Vec<PathBuf> = session_modified_files.iter().map(PathBuf::from).collect();

    let mut warnings = Vec::new();
    for handoff_file in ready_handoffs {
        let Ok(parsed) = extract_paths(&handoff_file) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(&handoff_file) else {
            continue;
        };

        let unchecked_items = parsed
            .checklist_items
            .iter()
            .filter(|item| !item.checked)
            .filter_map(|item| {
                let item_paths = referenced_paths_for_item(&content, item);
                let overlapping_files = item_paths
                    .iter()
                    .filter(|referenced| {
                        let referenced = Path::new(referenced);
                        session_paths.iter().any(|modified| {
                            modified.ends_with(referenced) || referenced.ends_with(modified)
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();

                (!overlapping_files.is_empty())
                    .then(|| (format_checklist_item(item), overlapping_files))
            })
            .collect::<Vec<_>>();
        if unchecked_items.is_empty() {
            continue;
        }

        let overlapping_files = unchecked_items
            .iter()
            .flat_map(|(_, overlaps)| overlaps.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        warnings.push(StaleHandoffWarning {
            handoff_file: handoff_file.clone(),
            overlapping_files,
            unchecked_items: unchecked_items.into_iter().map(|(item, _)| item).collect(),
            suggestion:
                "update the handoff checklist or mark the completed items before dispatching again"
                    .to_string(),
        });
    }

    warnings
}

fn check_handoff_completion(session_files: &[PathBuf], cwd: &Path) -> Vec<String> {
    let Some(workspace_root) = workspace_root_from_cwd(cwd) else {
        return Vec::new();
    };

    session_files
        .iter()
        .filter_map(|path| resolve_modified_handoff_path(path, cwd, &workspace_root))
        .filter_map(|handoff_path| {
            let audit = lint_handoff(&handoff_path).ok()?;
            let mut issues = Vec::new();

            if !audit.unchecked_checkboxes.is_empty() {
                issues.push(format!(
                    "unchecked checklist items at {}",
                    audit.unchecked_checkboxes
                        .iter()
                        .map(format_checklist_item)
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
            if !audit.empty_paste_markers.is_empty() {
                issues.push(format!(
                    "empty paste markers at lines {}",
                    audit.empty_paste_markers
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            (!issues.is_empty()).then(|| {
                format!(
                    "cortina: handoff completion warning for {}: {}; update the handoff before claiming completion",
                    handoff_path.display(),
                    issues.join("; ")
                )
            })
        })
        .collect()
}

fn ready_handoff_files(workspace_root: &Path) -> Vec<PathBuf> {
    let index_path = workspace_root.join(".handoffs/HANDOFFS.md");
    let Ok(content) = std::fs::read_to_string(index_path) else {
        return Vec::new();
    };

    content
        .lines()
        .filter_map(parse_ready_handoff_row)
        .map(|relative| workspace_root.join(".handoffs").join(relative))
        .filter(|path| path.exists())
        .collect()
}

fn parse_ready_handoff_row(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.contains("| Ready |") {
        return None;
    }
    let start = trimmed.find('(')? + 1;
    let end = trimmed[start..].find(')')? + start;
    Some(trimmed[start..end].to_string())
}

fn workspace_root_from_cwd(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|ancestor| ancestor.join(".handoffs/HANDOFFS.md").exists())
        .map(Path::to_path_buf)
}

fn format_checklist_item(item: &ChecklistItem) -> String {
    format!("line {}: {}", item.line_number, item.text)
}

fn referenced_paths_for_item(content: &str, item: &ChecklistItem) -> Vec<String> {
    let mut paths = extract_paths_from_text(&step_block_for_line(content, item.line_number));
    paths.extend(extract_paths_from_text(&item.text));
    paths.sort();
    paths.dedup();
    paths
}

fn step_block_for_line(content: &str, line_number: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let index = line_number.saturating_sub(1).min(lines.len() - 1);
    let mut start = 0;
    for cursor in (0..=index).rev() {
        if lines[cursor].trim_start().starts_with("### Step ") {
            start = cursor;
            break;
        }
    }

    let mut end = lines.len();
    for (cursor, line) in lines.iter().enumerate().skip(index + 1) {
        if line.trim_start().starts_with("### Step ") {
            end = cursor;
            break;
        }
    }

    lines[start..end].join("\n")
}

fn resolve_modified_handoff_path(
    session_file: &Path,
    cwd: &Path,
    workspace_root: &Path,
) -> Option<PathBuf> {
    if session_file.extension().is_none_or(|ext| ext != "md") {
        return None;
    }

    let candidates = if session_file.is_absolute() {
        vec![session_file.to_path_buf()]
    } else {
        vec![
            cwd.join(session_file),
            workspace_root.join(session_file),
            workspace_root.join(session_file.strip_prefix("./").unwrap_or(session_file)),
        ]
    };

    candidates.into_iter().find(|candidate| {
        candidate.exists()
            && candidate
                .components()
                .any(|component| component.as_os_str() == ".handoffs")
    })
}

/// Read transcript file text. Returns empty string on any error.
fn read_transcript_text(path: Option<&str>) -> String {
    let path = match path {
        Some(p) if !p.is_empty() => p,
        _ => return String::new(),
    };

    std::fs::read_to_string(path).unwrap_or_else(|e| {
        tracing::warn!("cortina: could not read transcript at {path}: {e}");
        String::new()
    })
}

/// Extract the final assistant message from transcript text.
/// Scans backwards through JSONL lines to find the last assistant entry.
fn extract_final_message(transcript_text: &str) -> String {
    for line in transcript_text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some("assistant") = entry.get("type").and_then(serde_json::Value::as_str) {
                if let Some(text) = entry.get("text").and_then(serde_json::Value::as_str) {
                    return text.to_string();
                }
            }
        }
    }

    String::new()
}

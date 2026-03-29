use std::path::Path;

use crate::events::{FileEditEvent, OutcomeEvent, OutcomeKind};
use crate::outcomes::record_outcome;
use crate::utils::{
    Importance, command_exists, current_timestamp_ms, ensure_scoped_hyphae_session,
    is_document_file, load_json_file, log_scoped_hyphae_feedback_signal, project_name_for_cwd,
    scope_hash, store_in_hyphae, update_json_file,
};

use super::{annotate_outcome_with_session, pending, truncate};

const CORRECTION_WINDOW_MS: u64 = 5 * 60 * 1000;
const CLEANUP_AGE_MS: u64 = 10 * 60 * 1000;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct EditEntry {
    file: String,
    old_string: String,
    new_string: String,
    timestamp: u64,
}

pub(super) fn handle_file_edits(event: &FileEditEvent) {
    let file_path = event.file_path.as_str();
    let old_str = event.old_string.as_str();
    let new_str = event.new_string.as_str();
    let scope_cwd = event.cwd.as_deref();

    if file_path.is_empty() {
        return;
    }

    let hash = scope_hash(scope_cwd);
    let track_file = crate::utils::temp_state_path("edits", &hash, "json");

    if !old_str.is_empty()
        && let Some(prev_edit) = find_correction(file_path, old_str, &track_file)
    {
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(file_path, 200))),
            OutcomeEvent::new(
                OutcomeKind::SelfCorrection,
                format!("Corrected recent edit in {}", truncate(file_path, 200)),
            )
            .with_file_path(truncate(file_path, 500)),
        );
        record_outcome(&hash, outcome);
        if command_exists("hyphae") {
            store_correction_in_hyphae(file_path, &prev_edit, old_str, new_str, scope_cwd);
        }
    }

    track_edit(&track_file, file_path, old_str, new_str);
    pending::track_pending_file(file_path, &hash);

    if is_document_file(file_path) {
        pending::track_pending_document(file_path, &hash);
    }

    if command_exists("hyphae") {
        let pending_docs = pending::take_pending_documents_batch(&hash);
        if !pending_docs.is_empty() {
            let failed_docs = pending::trigger_hyphae_ingest(&pending_docs, &hash, scope_cwd);
            if !failed_docs.is_empty() {
                pending::requeue_pending_documents(&failed_docs, &hash);
            }
        }
    }
}

fn load_and_clean_edits(track_file: &Path) -> Vec<EditEntry> {
    let mut edits: Vec<EditEntry> = load_json_file(track_file).unwrap_or_default();
    let cutoff = current_timestamp_ms().saturating_sub(CLEANUP_AGE_MS);
    edits.retain(|e| e.timestamp > cutoff);
    edits
}

fn track_edit(track_file: &Path, file_path: &str, old_str: &str, new_str: &str) {
    let now = current_timestamp_ms();
    let cutoff = now.saturating_sub(CLEANUP_AGE_MS);

    let _ = update_json_file::<Vec<EditEntry>, _, _>(track_file, |edits| {
        edits.retain(|e| e.timestamp > cutoff);
        edits.push(EditEntry {
            file: file_path.to_string(),
            old_string: old_str.chars().take(200).collect(),
            new_string: new_str.chars().take(200).collect(),
            timestamp: now,
        });
    });
}

fn find_correction(file_path: &str, old_str: &str, track_file: &Path) -> Option<EditEntry> {
    let edits = load_and_clean_edits(track_file);
    let cutoff = current_timestamp_ms().saturating_sub(CORRECTION_WINDOW_MS);

    edits
        .iter()
        .filter(|e| e.file == file_path && e.timestamp > cutoff && !e.new_string.is_empty())
        .find(|prev| prev.new_string.contains(old_str) || old_str.contains(&prev.new_string))
        .cloned()
}

fn store_correction_in_hyphae(
    file_path: &str,
    corrected_edit: &EditEntry,
    new_old_str: &str,
    new_new_str: &str,
    scope_cwd: Option<&str>,
) {
    let file_name = Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file_path);

    let content = format!(
        "File: {}\nOriginal change: {} → {}\nCorrection: {} → {}",
        file_name,
        corrected_edit.old_string,
        corrected_edit.new_string,
        &new_old_str[..new_old_str.len().min(200)],
        &new_new_str[..new_new_str.len().min(200)]
    );

    let project = project_name_for_cwd(scope_cwd);
    store_in_hyphae(
        "corrections",
        &content,
        Importance::High,
        project.as_deref(),
    );
    log_scoped_hyphae_feedback_signal(
        scope_cwd,
        "correction",
        -1,
        "cortina.post_tool_use.correction",
        Some(&truncate(file_path, 200)),
    );
}

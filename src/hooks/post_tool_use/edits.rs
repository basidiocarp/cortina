use std::path::Path;

use crate::events::{FileEditEvent, OutcomeEvent, OutcomeKind};
use crate::outcomes::{record_outcome, write_causal_signal};
use crate::policy::capture_policy;
use crate::utils::{
    Importance, command_exists, current_agent_id_for_cwd, current_timestamp_ms,
    ensure_scoped_hyphae_session, is_document_file, load_json_file,
    log_scoped_hyphae_feedback_signal, project_name_for_cwd, scope_hash, store_in_hyphae,
    update_json_file,
};

use super::{annotate_outcome_with_session, pending, truncate};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct EditEntry {
    file: String,
    old_string: String,
    new_string: String,
    timestamp: u64,
    session_id: Option<String>,
    project: Option<String>,
    project_root: Option<String>,
    worktree_id: Option<String>,
}

impl EditEntry {
    fn causal_signal(&self) -> crate::events::CausalSignal {
        let mut signal = write_causal_signal(
            &self.file,
            &format!(
                "Original edit in {}: {} -> {}",
                truncate(&self.file, 200),
                truncate(&self.old_string, 100),
                truncate(&self.new_string, 100)
            ),
            self.timestamp,
            None,
        );
        signal.session_id.clone_from(&self.session_id);
        signal.project.clone_from(&self.project);
        signal.project_root.clone_from(&self.project_root);
        signal.worktree_id.clone_from(&self.worktree_id);
        signal
    }
}

pub(super) fn handle_file_edits(event: &FileEditEvent) {
    let file_path = event.file_path.as_str();
    let old_str = event.old_string.as_str();
    let new_str = event.new_string.as_str();
    let scope_cwd = event.cwd.as_deref();

    if file_path.is_empty() {
        return;
    }

    let session = if command_exists("hyphae") {
        ensure_scoped_hyphae_session(scope_cwd, Some(&truncate(file_path, 200)))
    } else {
        None
    };
    let project = project_name_for_cwd(scope_cwd);

    let hash = scope_hash(scope_cwd);
    let track_file = crate::utils::temp_state_path("edits", &hash, "json");

    if !old_str.is_empty()
        && let Some(prev_edit) = find_correction(file_path, old_str, &track_file)
    {
        let caused_by = prev_edit.causal_signal();
        let outcome = annotate_outcome_with_session(
            session.clone(),
            OutcomeEvent::new(
                OutcomeKind::SelfCorrection,
                format!("Corrected recent edit in {}", truncate(file_path, 200)),
            )
            .with_file_path(truncate(file_path, 500)),
        )
        .with_caused_by(caused_by);
        let inserted = record_outcome(&hash, outcome);
        if inserted && command_exists("hyphae") {
            store_correction_in_hyphae(file_path, &prev_edit, old_str, new_str, scope_cwd);
        }
    }

    track_edit(
        &track_file,
        file_path,
        old_str,
        new_str,
        session.as_ref(),
        project,
    );
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
    let cutoff = current_timestamp_ms().saturating_sub(capture_policy().edit_cleanup_age_ms);
    edits.retain(|e| e.timestamp > cutoff);
    edits
}

fn track_edit(
    track_file: &Path,
    file_path: &str,
    old_str: &str,
    new_str: &str,
    session: Option<&crate::utils::SessionState>,
    project: Option<String>,
) {
    let now = current_timestamp_ms();
    let cutoff = now.saturating_sub(capture_policy().edit_cleanup_age_ms);

    let _ = update_json_file::<Vec<EditEntry>, _, _>(track_file, |edits| {
        edits.retain(|e| e.timestamp > cutoff);
        edits.push(EditEntry {
            file: file_path.to_string(),
            old_string: old_str.chars().take(200).collect(),
            new_string: new_str.chars().take(200).collect(),
            timestamp: now,
            session_id: session.map(|value| value.session_id.clone()),
            project,
            project_root: session.and_then(|value| value.project_root.clone()),
            worktree_id: session.and_then(|value| value.worktree_id.clone()),
        });
    });
}

fn find_correction(file_path: &str, old_str: &str, track_file: &Path) -> Option<EditEntry> {
    let edits = load_and_clean_edits(track_file);
    let cutoff = current_timestamp_ms().saturating_sub(capture_policy().correction_window_ms);

    edits
        .iter()
        .rev()
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
        truncate(new_old_str, 200),
        truncate(new_new_str, 200)
    );

    let project = project_name_for_cwd(scope_cwd);
    let agent_id = current_agent_id_for_cwd(scope_cwd);
    store_in_hyphae(
        "corrections",
        &content,
        Importance::High,
        project.as_deref(),
        agent_id.as_deref(),
    );
    log_scoped_hyphae_feedback_signal(
        scope_cwd,
        "correction",
        -1,
        "cortina.post_tool_use.correction",
        Some(&truncate(file_path, 200)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_entry_causal_signal_preserves_original_session_identity() {
        let entry = EditEntry {
            file: "src/lib.rs".to_string(),
            old_string: "old".to_string(),
            new_string: "new".to_string(),
            timestamp: 77,
            session_id: Some("ses-original".to_string()),
            project: Some("demo".to_string()),
            project_root: Some("/tmp/demo".to_string()),
            worktree_id: Some("git:demo".to_string()),
        };

        let signal = entry.causal_signal();

        assert_eq!(signal.signal_kind, "write");
        assert_eq!(signal.file_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(signal.session_id.as_deref(), Some("ses-original"));
        assert_eq!(signal.project.as_deref(), Some("demo"));
        assert_eq!(signal.project_root.as_deref(), Some("/tmp/demo"));
        assert_eq!(signal.worktree_id.as_deref(), Some("git:demo"));
    }

    #[test]
    fn find_correction_prefers_most_recent_matching_edit() {
        let track_file = crate::utils::temp_state_path(
            "edit-correction-test",
            &format!("find-correction-{}", std::process::id()),
            "json",
        );
        let _ = std::fs::remove_file(&track_file);

        let now = current_timestamp_ms();
        let older = EditEntry {
            file: "src/lib.rs".to_string(),
            old_string: "fn v1() {}".to_string(),
            new_string: "fn current_candidate() {}".to_string(),
            timestamp: now.saturating_sub(50),
            session_id: Some("ses-old".to_string()),
            project: Some("demo".to_string()),
            project_root: None,
            worktree_id: None,
        };
        let newer = EditEntry {
            file: "src/lib.rs".to_string(),
            old_string: "fn v2() {}".to_string(),
            new_string: "fn current_candidate() {}".to_string(),
            timestamp: now,
            session_id: Some("ses-new".to_string()),
            project: Some("demo".to_string()),
            project_root: None,
            worktree_id: None,
        };

        crate::utils::save_json_file(&track_file, &vec![older, newer]).expect("write edits");

        let matched = find_correction("src/lib.rs", "fn current_candidate() {}", &track_file)
            .expect("matching edit");

        assert_eq!(matched.session_id.as_deref(), Some("ses-new"));
        assert_eq!(matched.timestamp, now);

        let _ = std::fs::remove_file(track_file);
    }
}

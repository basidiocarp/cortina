use std::path::{Path, PathBuf};

use crate::events::{OutcomeEvent, OutcomeKind};
use crate::outcomes::record_outcome;
use crate::policy::capture_policy;
use crate::utils::{
    command_exists, ensure_scoped_hyphae_session, spawn_async_checked, temp_state_path,
    update_json_file,
};
#[cfg(test)]
use crate::utils::{load_json_file, remove_file_with_lock};

use super::annotate_outcome_with_session;

const HYPHAE_INGEST_LABEL: &str = "hyphae ingest";
const HYPHAE_INGEST_SUBCOMMAND: &str = "ingest";

fn pending_files_path(hash: &str) -> PathBuf {
    temp_state_path("pending-exports", hash, "json")
}

fn pending_documents_path(hash: &str) -> PathBuf {
    temp_state_path("pending-ingest", hash, "json")
}

#[cfg(test)]
pub(super) fn get_pending_files(hash: &str) -> Vec<String> {
    load_json_file(pending_files_path(hash)).unwrap_or_default()
}

#[cfg(test)]
pub(super) fn get_pending_documents(hash: &str) -> Vec<String> {
    load_json_file(pending_documents_path(hash)).unwrap_or_default()
}

pub(super) fn hyphae_ingest_args(document: &str) -> [&str; 2] {
    [HYPHAE_INGEST_SUBCOMMAND, document]
}

pub(super) fn track_pending_file(file_path: &str, hash: &str) {
    let path = pending_files_path(hash);
    let _ = update_json_file::<Vec<String>, _, _>(&path, |files| {
        if !files.iter().any(|existing| existing == file_path) {
            files.push(file_path.to_string());
        }
    });
}

pub(super) fn track_pending_document(file_path: &str, hash: &str) {
    let path = pending_documents_path(hash);
    let _ = update_json_file::<Vec<String>, _, _>(&path, |files| {
        if !files.iter().any(|existing| existing == file_path) {
            files.push(file_path.to_string());
        }
    });
}

#[cfg(test)]
pub(super) fn clear_pending_files(hash: &str) {
    let path = pending_files_path(hash);
    let _ = remove_file_with_lock(&path);
}

#[cfg(test)]
pub(super) fn clear_pending_documents(hash: &str) {
    let path = pending_documents_path(hash);
    let _ = remove_file_with_lock(&path);
}

pub(super) fn check_and_trigger_exports(hash: &str, scope_cwd: Option<&str>) {
    if !command_exists("rhizome") {
        return;
    }

    let pending_files = take_pending_files_batch(hash);
    if !pending_files.is_empty() {
        if !spawn_async_checked("rhizome", &["export"]) {
            requeue_pending_files(&pending_files, hash);
            return;
        }
        let outcome = annotate_outcome_with_session(
            ensure_scoped_hyphae_session(scope_cwd, Some("rhizome export")),
            OutcomeEvent::new(
                OutcomeKind::KnowledgeExported,
                format!("Triggered rhizome export for {} files", pending_files.len()),
            )
            .with_command("rhizome export"),
        );
        let _ = record_outcome(hash, outcome);
    }
}

pub(super) fn trigger_hyphae_ingest(
    documents: &[String],
    hash: &str,
    scope_cwd: Option<&str>,
) -> Vec<String> {
    let mut spawned = 0usize;
    let mut failed_docs = Vec::new();
    for doc in documents {
        if spawn_async_checked("hyphae", &hyphae_ingest_args(doc)) {
            spawned += 1;
        } else {
            failed_docs.push(doc.clone());
        }
    }
    if spawned == 0 {
        return failed_docs;
    }
    let outcome = annotate_outcome_with_session(
        ensure_scoped_hyphae_session(scope_cwd, Some(HYPHAE_INGEST_LABEL)),
        OutcomeEvent::new(
            OutcomeKind::DocumentIngested,
            format!("Triggered hyphae ingest for {spawned} documents"),
        )
        .with_command(HYPHAE_INGEST_LABEL),
    );
    let _ = record_outcome(hash, outcome);
    failed_docs
}

pub(super) fn take_pending_documents_batch(hash: &str) -> Vec<String> {
    take_pending_batch(
        &pending_documents_path(hash),
        capture_policy().ingest_threshold,
    )
}

fn take_pending_files_batch(hash: &str) -> Vec<String> {
    take_pending_batch(&pending_files_path(hash), capture_policy().export_threshold)
}

fn take_pending_batch(path: &Path, threshold: usize) -> Vec<String> {
    update_json_file::<Vec<String>, _, _>(path, |entries| {
        if entries.len() < threshold {
            Vec::new()
        } else {
            std::mem::take(entries)
        }
    })
    .unwrap_or_default()
}

fn requeue_pending_files(files: &[String], hash: &str) {
    requeue_pending_batch(&pending_files_path(hash), files);
}

pub(super) fn requeue_pending_documents(files: &[String], hash: &str) {
    requeue_pending_batch(&pending_documents_path(hash), files);
}

fn requeue_pending_batch(path: &Path, files: &[String]) {
    let _ = update_json_file::<Vec<String>, _, _>(path, |entries| {
        for file in files {
            if !entries.iter().any(|existing| existing == file) {
                entries.push(file.clone());
            }
        }
    });
}

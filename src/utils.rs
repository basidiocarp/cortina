mod canopy_client;
mod command_signals;
mod hyphae_client;
mod session_scope;
mod state;
#[cfg(test)]
mod tests;

#[cfg(not(test))]
pub(crate) use canopy_client::attach_outcome_evidence;
pub(crate) use canopy_client::{
    current_task_id_for_cwd, evidence_bridge_stats, evidence_bridge_stats_path,
};
#[cfg(test)]
pub(crate) use canopy_client::{note_evidence_write_failure, note_evidence_write_success};
#[cfg(test)]
use command_signals::{SessionOutcome, is_test_command};
pub use command_signals::{
    has_error, is_build_command, is_document_file, is_significant_command, normalize_command,
    session_outcome_feedback, successful_validation_feedback,
};
pub(crate) use hyphae_client::resolved_command;
pub use hyphae_client::{Importance, command_exists, spawn_async_checked, store_in_hyphae};
pub use session_scope::{
    SessionState, end_scoped_hyphae_session, ensure_scoped_hyphae_session, load_session_state,
    log_hyphae_feedback_signal_for_session, log_scoped_hyphae_feedback_signal,
    project_name_for_cwd, scoped_session_liveness,
};
pub(crate) use session_scope::scope_identity_for_cwd;
#[cfg(test)]
pub use state::save_json_file;
pub use state::{
    current_timestamp_ms, load_json_file, remove_file_with_lock, scope_hash, temp_state_path,
    update_json_file,
};

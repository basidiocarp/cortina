mod command_signals;
mod hyphae_client;
mod session_scope;
mod state;
#[cfg(test)]
mod tests;

#[cfg(test)]
use command_signals::{SessionOutcome, is_test_command};
pub use command_signals::{
    has_error, is_build_command, is_document_file, is_significant_command, normalize_command,
    session_outcome_feedback, successful_validation_feedback,
};
pub use hyphae_client::{Importance, command_exists, spawn_async_checked, store_in_hyphae};
pub use session_scope::{
    SessionState, end_scoped_hyphae_session, ensure_scoped_hyphae_session, load_session_state,
    log_hyphae_feedback_signal_for_session, log_scoped_hyphae_feedback_signal,
    project_name_for_cwd, scoped_session_liveness,
};
pub use state::{
    current_timestamp_ms, load_json_file, remove_file_with_lock, scope_hash, temp_state_path,
    update_json_file,
};
#[cfg(test)]
pub use state::save_json_file;

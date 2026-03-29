use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use super::hyphae_client::command_exists;
use super::state::{
    current_timestamp_ms, load_json_file, save_json_file, scope_hash, temp_state_path,
    with_file_lock, with_lock_path,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub project: String,
    #[serde(default)]
    pub started_at: u64,
}

pub fn load_session_state(hash: &str) -> Option<SessionState> {
    load_json_file(session_state_path(hash))
}

pub fn scoped_session_liveness(cwd: Option<&str>) -> Option<bool> {
    if !command_exists("hyphae") {
        return None;
    }

    let hash = scope_hash(cwd);
    let state = load_session_state(&hash)?;
    Some(is_cached_session_active(
        &hash,
        &state.project,
        &state.session_id,
        Command::output,
    ))
}

#[cfg(test)]
pub(super) fn clear_session_state(hash: &str) {
    let path = session_state_path(hash);
    let _ = super::state::remove_file_with_lock(path);
}

pub fn project_name_for_cwd(cwd: Option<&str>) -> Option<String> {
    cwd.map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

pub fn ensure_scoped_hyphae_session(cwd: Option<&str>, task: Option<&str>) -> Option<SessionState> {
    if !command_exists("hyphae") {
        return None;
    }

    let project = project_name_for_cwd(cwd)?;
    ensure_hyphae_session_with_hash(&scope_hash(cwd), &project, task, Command::output)
}

pub(super) fn ensure_hyphae_session_with_hash<F>(
    hash: &str,
    project: &str,
    task: Option<&str>,
    mut run_command: F,
) -> Option<SessionState>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let path = session_state_path(hash);
    with_session_operation_lock(hash, || {
        let existing = with_file_lock(&path, || Ok(load_json_file::<SessionState>(&path)))?;

        if let Some(existing) = existing {
            if is_cached_session_active(hash, project, &existing.session_id, &mut run_command) {
                return Ok(Some(existing));
            }

            let new_state = start_hyphae_session(hash, project, task, &mut run_command)?;
            return with_file_lock(&path, || match load_json_file::<SessionState>(&path) {
                Some(current) if current.session_id != existing.session_id => Ok(Some(current)),
                _ => {
                    save_json_file(&path, &new_state)?;
                    Ok(Some(new_state.clone()))
                }
            });
        }

        let new_state = start_hyphae_session(hash, project, task, &mut run_command)?;
        with_file_lock(&path, || {
            if let Some(current) = load_json_file::<SessionState>(&path) {
                return Ok(Some(current));
            }

            save_json_file(&path, &new_state)?;
            Ok(Some(new_state))
        })
    })
    .ok()
    .flatten()
}

pub fn end_scoped_hyphae_session(
    cwd: Option<&str>,
    summary: Option<&str>,
    files_modified: &[String],
    errors_encountered: usize,
) -> Option<SessionState> {
    if !command_exists("hyphae") {
        return None;
    }

    end_hyphae_session_with(
        scope_hash(cwd).as_str(),
        summary,
        files_modified,
        errors_encountered,
        Command::output,
    )
}

pub(super) fn end_hyphae_session_with<F>(
    hash: &str,
    summary: Option<&str>,
    files_modified: &[String],
    errors_encountered: usize,
    mut run_command: F,
) -> Option<SessionState>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    if hash.is_empty() {
        return None;
    }

    let path = session_state_path(hash);
    with_session_operation_lock(hash, || {
        let state = with_file_lock(&path, || Ok(load_json_file::<SessionState>(&path)))?;
        let Some(state) = state else {
            return Ok(None);
        };

        let mut cmd = Command::new("hyphae");
        cmd.args(["session", "end", "--id", &state.session_id]);

        if let Some(summary_text) = summary.filter(|value| !value.trim().is_empty()) {
            cmd.args(["--summary", summary_text]);
        }

        for file in files_modified {
            cmd.args(["--file", file]);
        }

        cmd.args(["--errors", &errors_encountered.to_string()]);

        let Ok(output) = run_command(&mut cmd) else {
            return Ok(None);
        };

        if !output.status.success() {
            return Ok(None);
        }

        with_file_lock(&path, || {
            if load_json_file::<SessionState>(&path)
                .as_ref()
                .is_some_and(|current| current.session_id == state.session_id)
            {
                let _ = fs::remove_file(&path);
            }
            Ok(Some(state))
        })
    })
    .ok()
    .flatten()
}

pub fn log_scoped_hyphae_feedback_signal(
    cwd: Option<&str>,
    signal_type: &str,
    signal_value: i64,
    source: &str,
    task: Option<&str>,
) {
    if !command_exists("hyphae") {
        return;
    }

    let hash = scope_hash(cwd);
    let state = load_session_state(&hash).or_else(|| ensure_scoped_hyphae_session(cwd, task));
    let Some(state) = state else {
        return;
    };

    log_hyphae_feedback_signal_for_session(&state, signal_type, signal_value, source);
}

pub fn log_hyphae_feedback_signal_for_session(
    state: &SessionState,
    signal_type: &str,
    signal_value: i64,
    source: &str,
) {
    if !command_exists("hyphae") {
        return;
    }

    let mut cmd = Command::new("hyphae");
    cmd.args(["feedback", "signal"])
        .args(["--session-id", &state.session_id])
        .args(["--type", signal_type])
        .args(["--value", &signal_value.to_string()])
        .args(["--source", source])
        .args(["--project", &state.project]);

    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub(super) fn session_state_path(hash: &str) -> PathBuf {
    temp_state_path("session", hash, "json")
}

fn session_operation_lock_path(hash: &str) -> PathBuf {
    temp_state_path("session-op", hash, "json.lock")
}

fn with_session_operation_lock<R>(hash: &str, operation: impl FnOnce() -> Result<R>) -> Result<R> {
    with_lock_path(&session_operation_lock_path(hash), true, operation)
}

fn start_hyphae_session<F>(
    hash: &str,
    project: &str,
    task: Option<&str>,
    run_command: &mut F,
) -> Result<SessionState>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let mut cmd = Command::new("hyphae");
    cmd.args(["session", "start", "--project", project, "--scope", hash]);
    if let Some(task_desc) = task.filter(|value| !value.trim().is_empty()) {
        cmd.args(["--task", task_desc]);
    }

    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("hyphae session start failed"));
    }

    let session_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if session_id.is_empty() {
        return Err(anyhow::anyhow!(
            "hyphae session start returned empty session id"
        ));
    }

    Ok(SessionState {
        session_id,
        project: project.to_string(),
        started_at: current_timestamp_ms(),
    })
}

fn is_cached_session_active<F>(
    hash: &str,
    project: &str,
    session_id: &str,
    mut run_command: F,
) -> bool
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let mut cmd = Command::new("hyphae");
    cmd.args(["session", "status", "--id", session_id]);

    let Ok(output) = run_command(&mut cmd) else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let Ok(parsed) = serde_json::from_slice::<Value>(&output.stdout) else {
        return false;
    };

    parsed
        .get("session_id")
        .and_then(Value::as_str)
        .is_some_and(|value| value == session_id)
        && parsed
            .get("project")
            .and_then(Value::as_str)
            .is_some_and(|value| value == project)
        && parsed
            .get("scope")
            .and_then(Value::as_str)
            .is_some_and(|value| value == hash)
        && parsed
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

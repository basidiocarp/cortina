use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spore::logging::{SpanContext, subprocess_span, tool_span};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tracing::{debug, warn};

use super::hyphae_client::{command_exists, resolved_command};
use super::state::{
    canonicalize_path, current_runtime_session_id, current_timestamp_ms, load_json_file,
    save_json_file, scope_hash, stable_identity_hash, temp_state_path, with_file_lock,
    with_lock_path,
};
use crate::events::OutcomeKind;
use crate::outcomes::load_outcomes;

fn span_context(project_root: Option<&str>, tool: &str) -> SpanContext {
    let context = SpanContext::for_app("cortina").with_tool(tool);
    match project_root {
        Some(project_root) if !project_root.trim().is_empty() => {
            context.with_workspace_root(project_root.to_string())
        }
        _ => context,
    }
}

fn diagnostic_stderr() -> std::process::Stdio {
    #[cfg(test)]
    {
        std::process::Stdio::null()
    }
    #[cfg(not(test))]
    {
        std::process::Stdio::inherit()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(
        default,
        rename = "_legacy_scope",
        skip_serializing_if = "Option::is_none"
    )]
    pub legacy_scope: Option<String>,
    #[serde(default)]
    pub started_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionIdentity {
    pub project: String,
    pub project_root: String,
    pub worktree_id: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSessionMatch {
    project_root: String,
    worktree_id: String,
}

impl SessionState {
    fn new(session_id: String, identity: &SessionIdentity) -> Self {
        Self {
            session_id,
            project: identity.project.clone(),
            project_root: Some(identity.project_root.clone()),
            worktree_id: Some(identity.worktree_id.clone()),
            legacy_scope: None,
            started_at: current_timestamp_ms(),
        }
    }
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
    let identity = session_identity_for_cwd(cwd)?;
    Some(match_active_session(&identity, &state.session_id, Command::output).is_some())
}

#[cfg(test)]
pub(super) fn clear_session_state(hash: &str) {
    let path = session_state_path(hash);
    let _ = super::state::remove_file_with_lock(path);
}

pub fn project_name_for_cwd(cwd: Option<&str>) -> Option<String> {
    session_identity_for_cwd(cwd).map(|identity| identity.project)
}

pub fn ensure_scoped_hyphae_session(cwd: Option<&str>, task: Option<&str>) -> Option<SessionState> {
    if !command_exists("hyphae") {
        return None;
    }

    let identity = session_identity_for_cwd(cwd)?;
    let hash = scope_hash(cwd);
    ensure_hyphae_session_with_hash(&hash, &identity, task, Command::output)
}

#[allow(
    dead_code,
    reason = "Test-only helper for injecting hash-based session state runners"
)]
pub(super) fn ensure_hyphae_session_with_hash<F>(
    hash: &str,
    identity: &SessionIdentity,
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
            if let Some(active_session) =
                match_active_session(identity, &existing.session_id, &mut run_command)
            {
                let mut current = existing.clone();
                current.project.clone_from(&identity.project);
                current.project_root = Some(active_session.project_root);
                current.worktree_id = Some(active_session.worktree_id);
                current.legacy_scope = None;
                return with_file_lock(&path, || {
                    if current != existing {
                        save_json_file(&path, &current)?;
                    }
                    Ok(Some(current))
                });
            }

            let context_signals =
                collect_context_signals(hash, &identity.project_root, &mut run_command);
            let new_state =
                start_hyphae_session(identity, task, &context_signals, &mut run_command)?;
            return with_file_lock(&path, || match load_json_file::<SessionState>(&path) {
                Some(current) if current.session_id != existing.session_id => Ok(Some(current)),
                _ => {
                    save_json_file(&path, &new_state)?;
                    Ok(Some(new_state.clone()))
                }
            });
        }

        let context_signals =
            collect_context_signals(hash, &identity.project_root, &mut run_command);
        let new_state = start_hyphae_session(identity, task, &context_signals, &mut run_command)?;
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
        let context = span_context(state.project_root.as_deref(), "hyphae_session_end");
        let _tool_span = tool_span("hyphae_session_end", &context).entered();

        let Some(mut cmd) = resolved_command("hyphae") else {
            warn!("Hyphae binary is not discoverable; cannot end scoped session");
            return Ok(None);
        };
        cmd.args(["session", "end", "--id", &state.session_id]);

        if let Some(summary_text) = summary.filter(|value| !value.trim().is_empty()) {
            cmd.args(["--summary", summary_text]);
        }

        for file in files_modified {
            cmd.args(["--file", file]);
        }

        cmd.args(["--errors", &errors_encountered.to_string()]);

        let _subprocess_span = subprocess_span("hyphae session end", &context).entered();
        let Ok(output) = run_command(&mut cmd) else {
            warn!("Failed to execute hyphae session end");
            return Ok(None);
        };

        if !output.status.success() {
            warn!(
                "Hyphae session end exited non-zero for session {}",
                state.session_id
            );
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

    let context = span_context(state.project_root.as_deref(), "hyphae_feedback_signal");
    let _tool_span = tool_span("hyphae_feedback_signal", &context).entered();
    let Some(mut cmd) = resolved_command("hyphae") else {
        warn!("Hyphae binary is not discoverable; cannot log feedback signal");
        return;
    };
    cmd.args(["feedback", "signal"])
        .args(["--session-id", &state.session_id])
        .args(["--type", signal_type])
        .args(["--value", &signal_value.to_string()])
        .args(["--source", source])
        .args(["--project", &state.project]);

    let _spawn_span = subprocess_span("hyphae feedback signal", &context).entered();
    if let Err(err) = cmd
        .stdout(std::process::Stdio::null())
        .stderr(diagnostic_stderr())
        .spawn()
    {
        warn!("Failed to spawn hyphae feedback signal command: {err}");
    }
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

#[derive(Debug, Clone, Default)]
struct ContextSignals {
    recent_files: Vec<String>,
    active_errors: Vec<String>,
    git_branch: Option<String>,
}

fn collect_context_signals<F>(hash: &str, project_root: &str, run_command: &mut F) -> ContextSignals
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    ContextSignals {
        recent_files: collect_recent_files(hash),
        active_errors: collect_active_errors(hash),
        git_branch: git_branch_for_workspace(project_root, run_command),
    }
}

fn collect_recent_files(hash: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    let edit_entries =
        load_json_file::<Vec<Value>>(temp_state_path("edits", hash, "json")).unwrap_or_default();
    for entry in edit_entries.iter().rev() {
        let Some(file) = entry.get("file").and_then(Value::as_str) else {
            continue;
        };
        if !files.iter().any(|existing| existing == file) {
            files.push(file.to_string());
        }
        if files.len() >= 5 {
            return files;
        }
    }

    for path in [
        temp_state_path("pending-exports", hash, "json"),
        temp_state_path("pending-ingest", hash, "json"),
    ] {
        let entries = load_json_file::<Vec<String>>(path).unwrap_or_default();
        for entry in entries.iter().rev() {
            if !files.iter().any(|existing| existing == entry) {
                files.push(entry.clone());
            }
            if files.len() >= 5 {
                return files;
            }
        }
    }

    files
}

fn collect_active_errors(hash: &str) -> Vec<String> {
    let mut errors = Vec::new();
    for outcome in load_outcomes(hash).into_iter().rev() {
        if outcome.kind != OutcomeKind::ErrorDetected {
            continue;
        }
        if !outcome.summary.trim().is_empty()
            && !errors.iter().any(|existing| existing == &outcome.summary)
        {
            errors.push(outcome.summary);
        }
        if errors.len() >= 3 {
            break;
        }
    }
    errors
}

fn git_branch_for_workspace<F>(project_root: &str, run_command: &mut F) -> Option<String>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let context = span_context(Some(project_root), "git_branch_for_workspace");
    let _subprocess_span = subprocess_span("git rev-parse --abbrev-ref HEAD", &context).entered();
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(project_root);
    let output = run_command(&mut cmd).ok()?;
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if branch.is_empty() || matches!(branch.as_str(), "HEAD" | "main" | "master" | "develop") {
        None
    } else {
        Some(branch)
    }
}

fn start_hyphae_session<F>(
    identity: &SessionIdentity,
    task: Option<&str>,
    context_signals: &ContextSignals,
    run_command: &mut F,
) -> Result<SessionState>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let context = span_context(Some(&identity.project_root), "hyphae_session_start");
    let _tool_span = tool_span("hyphae_session_start", &context).entered();
    let Some(mut cmd) = resolved_command("hyphae") else {
        return Err(anyhow::anyhow!("hyphae is not discoverable"));
    };
    cmd.args(["session", "start", "--project", &identity.project])
        .args(["--project-root", &identity.project_root])
        .args(["--worktree-id", &identity.worktree_id])
        .args(["--scope", &identity.scope]);
    if let Some(runtime_session_id) = current_runtime_session_id() {
        cmd.args(["--runtime-session-id", &runtime_session_id]);
    }
    if let Some(task_desc) = task.filter(|value| !value.trim().is_empty()) {
        cmd.args(["--task", task_desc]);
    }

    for file in &context_signals.recent_files {
        cmd.args(["--recent-files", file]);
    }
    for error in &context_signals.active_errors {
        cmd.args(["--active-errors", error]);
    }
    if let Some(branch) = &context_signals.git_branch {
        cmd.args(["--git-branch", branch]);
    }

    let _subprocess_span = subprocess_span("hyphae session start", &context).entered();
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        warn!(
            "Hyphae session start exited non-zero for project {}",
            identity.project
        );
        return Err(anyhow::anyhow!("hyphae session start failed"));
    }

    let session_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if session_id.is_empty() {
        return Err(anyhow::anyhow!(
            "hyphae session start returned empty session id"
        ));
    }

    Ok(SessionState::new(session_id, identity))
}

fn match_active_session<F>(
    identity: &SessionIdentity,
    session_id: &str,
    mut run_command: F,
) -> Option<ActiveSessionMatch>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let context = span_context(Some(&identity.project_root), "hyphae_session_status");
    let _tool_span = tool_span("hyphae_session_status", &context).entered();
    let mut cmd = resolved_command("hyphae")?;
    cmd.args(["session", "status", "--id", session_id]);

    let _subprocess_span = subprocess_span("hyphae session status", &context).entered();
    let Ok(output) = run_command(&mut cmd) else {
        debug!("Failed to execute hyphae session status for session {session_id}");
        return None;
    };

    if !output.status.success() {
        debug!("Hyphae session status returned non-success for session {session_id}");
        return None;
    }

    let Ok(parsed) = serde_json::from_slice::<Value>(&output.stdout) else {
        return None;
    };

    let session_matches = parsed
        .get("session_id")
        .and_then(Value::as_str)
        .is_some_and(|value| value == session_id)
        && parsed
            .get("project")
            .and_then(Value::as_str)
            .is_some_and(|value| value == identity.project)
        && parsed
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    if !session_matches {
        return None;
    }

    let project_root = parsed
        .get("project_root")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)?;
    let worktree_id = parsed
        .get("worktree_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)?;

    if project_root != identity.project_root || worktree_id != identity.worktree_id {
        return None;
    }

    Some(ActiveSessionMatch {
        project_root,
        worktree_id,
    })
}

fn session_identity_for_cwd(cwd: Option<&str>) -> Option<SessionIdentity> {
    session_identity_for_cwd_with(cwd, Command::output)
}

pub(crate) fn scope_identity_for_cwd(cwd: Option<&str>) -> Option<(String, String)> {
    let identity = session_identity_for_cwd(cwd)?;
    Some((identity.project_root, identity.worktree_id))
}

pub(super) fn session_identity_for_cwd_with<F>(
    cwd: Option<&str>,
    mut run_command: F,
) -> Option<SessionIdentity>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let scope = scope_hash(cwd);
    let cwd = resolved_cwd(cwd)?;
    let project_root = cwd.to_string_lossy().to_string();

    // Use the git dir path when available so linked worktrees get distinct stable ids.
    // Outside git, fall back to the canonical root path and mark the source explicitly.
    let worktree_id =
        git_command_output(&cwd, &["rev-parse", "--absolute-git-dir"], &mut run_command)
            .map(PathBuf::from)
            .map(canonicalize_path)
            .map_or_else(
                || format!("path:{}", stable_identity_hash(project_root.as_str())),
                |path| {
                    format!(
                        "git:{}",
                        stable_identity_hash(path.to_string_lossy().as_ref())
                    )
                },
            );

    Some(SessionIdentity {
        project: project_name_from_root(&cwd)?,
        project_root,
        worktree_id,
        scope,
    })
}

fn resolved_cwd(cwd: Option<&str>) -> Option<PathBuf> {
    cwd.map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .map(canonicalize_path)
}

fn project_name_from_root(root: &Path) -> Option<String> {
    root.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .or_else(|| {
            let text = root.to_string_lossy();
            (!text.trim().is_empty()).then_some(text.to_string())
        })
}

fn git_command_output<F>(cwd: &Path, args: &[&str], run_command: &mut F) -> Option<String>
where
    F: FnMut(&mut Command) -> std::io::Result<Output>,
{
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(args);
    let output = run_command(&mut cmd).ok()?;
    if !output.status.success() {
        return None;
    }

    let output = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!output.is_empty()).then_some(output)
}

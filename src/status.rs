use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::policy::{CapturePolicy, capture_policy};
#[cfg(test)]
use crate::utils::save_json_file;
use crate::utils::{
    SessionState, command_exists, load_json_file, load_session_state, scope_hash,
    scoped_session_liveness, temp_state_path,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatusReport {
    cwd: String,
    scope_hash: String,
    hyphae_available: bool,
    rhizome_available: bool,
    session: Option<SessionStatus>,
    session_live: Option<bool>,
    outcome_count: usize,
    volva_hook_event_count: usize,
    pending_export_count: usize,
    pending_ingest_count: usize,
    policy: CapturePolicy,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SessionStatus {
    session_id: String,
    project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worktree_id: Option<String>,
    started_at: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorReport {
    cwd: String,
    scope_hash: String,
    temp_dir: String,
    temp_dir_writable: bool,
    hyphae_available: bool,
    rhizome_available: bool,
    session_live: Option<bool>,
    session_state: FileHealth,
    outcomes: FileHealth,
    volva_hook_events: FileHealth,
    pending_exports: FileHealth,
    pending_ingest: FileHealth,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileHealth {
    path: String,
    exists: bool,
    valid_json: bool,
}

pub fn print_status(json: bool, cwd: Option<&str>) -> Result<()> {
    let report = collect_status(cwd);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Cortina status");
    println!("cwd={}", report.cwd);
    println!("scope_hash={}", report.scope_hash);
    println!("hyphae_available={}", report.hyphae_available);
    println!("rhizome_available={}", report.rhizome_available);
    match report.session {
        Some(session) => {
            println!("session_active=true");
            println!("session_id={}", session.session_id);
            println!("session_project={}", session.project);
            if let Some(project_root) = session.project_root.as_deref() {
                println!("session_project_root={project_root}");
            }
            if let Some(worktree_id) = session.worktree_id.as_deref() {
                println!("session_worktree_id={worktree_id}");
            }
            println!("session_started_at={}", session.started_at);
            if let Some(session_live) = report.session_live {
                println!("session_live={session_live}");
            }
        }
        None => println!("session_active=false"),
    }
    println!("outcome_count={}", report.outcome_count);
    println!("volva_hook_event_count={}", report.volva_hook_event_count);
    println!("pending_export_count={}", report.pending_export_count);
    println!("pending_ingest_count={}", report.pending_ingest_count);
    println!(
        "policy=dedupe:{}ms correction:{}ms cleanup:{}ms export:{} ingest:{} grace:{}ms max_outcomes:{} fallback_on_end_failure:{}",
        report.policy.outcome_dedupe_window_ms,
        report.policy.correction_window_ms,
        report.policy.edit_cleanup_age_ms,
        report.policy.export_threshold,
        report.policy.ingest_threshold,
        report.policy.outcome_attribution_grace_ms,
        report.policy.max_outcome_events,
        report.policy.fallback_session_memory_on_end_failure
    );
    Ok(())
}

pub fn print_doctor(json: bool, cwd: Option<&str>) -> Result<()> {
    let report = collect_doctor(cwd);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Cortina doctor");
    println!("cwd={}", report.cwd);
    println!("scope_hash={}", report.scope_hash);
    println!("temp_dir={}", report.temp_dir);
    println!("temp_dir_writable={}", report.temp_dir_writable);
    println!("hyphae_available={}", report.hyphae_available);
    println!("rhizome_available={}", report.rhizome_available);
    if let Some(session_live) = report.session_live {
        println!("session_live={session_live}");
    }
    print_file_health("session_state", &report.session_state);
    print_file_health("outcomes", &report.outcomes);
    print_file_health("volva_hook_events", &report.volva_hook_events);
    print_file_health("pending_exports", &report.pending_exports);
    print_file_health("pending_ingest", &report.pending_ingest);
    if report.warnings.is_empty() {
        println!("warnings=none");
    } else {
        for warning in &report.warnings {
            println!("warning={warning}");
        }
    }
    Ok(())
}

pub fn collect_status(cwd: Option<&str>) -> StatusReport {
    let cwd = normalized_cwd(cwd);
    let hash = scope_hash(Some(&cwd));
    let session_live = scoped_session_liveness(Some(&cwd));
    StatusReport {
        cwd,
        scope_hash: hash.clone(),
        hyphae_available: command_exists("hyphae"),
        rhizome_available: command_exists("rhizome"),
        session: load_session_state(&hash).map(SessionStatus::from),
        session_live,
        outcome_count: json_vec_len(&outcomes_path(&hash)),
        volva_hook_event_count: json_vec_len(&volva_hook_events_path(&hash)),
        pending_export_count: json_vec_len(&pending_exports_path(&hash)),
        pending_ingest_count: json_vec_len(&pending_ingest_path(&hash)),
        policy: capture_policy().clone(),
    }
}

pub fn collect_doctor(cwd: Option<&str>) -> DoctorReport {
    let cwd = normalized_cwd(cwd);
    let hash = scope_hash(Some(&cwd));
    let temp_dir = env::temp_dir();
    let temp_dir_writable = temp_dir_is_writable(&temp_dir);
    let hyphae_available = command_exists("hyphae");
    let rhizome_available = command_exists("rhizome");
    let session_live = scoped_session_liveness(Some(&cwd));
    let session_state = inspect_json_file(&session_state_path(&hash));
    let outcomes = inspect_json_file(&outcomes_path(&hash));
    let volva_hook_events = inspect_json_file(&volva_hook_events_path(&hash));
    let pending_exports = inspect_json_file(&pending_exports_path(&hash));
    let pending_ingest = inspect_json_file(&pending_ingest_path(&hash));

    let mut warnings = Vec::new();
    if !temp_dir_writable {
        warnings.push(format!(
            "temp dir is not writable: {}",
            temp_dir.to_string_lossy()
        ));
    }
    if !hyphae_available {
        warnings.push(
            "hyphae is not on PATH; structured capture and session persistence will be degraded"
                .to_string(),
        );
    }
    if !rhizome_available && pending_exports.exists {
        warnings.push("rhizome is not on PATH; pending export state cannot flush".to_string());
    }
    if !hyphae_available && pending_ingest.exists {
        warnings.push("hyphae is not on PATH; pending ingest state cannot flush".to_string());
    }
    if session_state.exists && session_state.valid_json && session_live == Some(false) {
        warnings.push(
            "cached session state exists but Hyphae reports the session is no longer active"
                .to_string(),
        );
    }
    for (label, health) in [
        ("session_state", &session_state),
        ("outcomes", &outcomes),
        ("volva_hook_events", &volva_hook_events),
        ("pending_exports", &pending_exports),
        ("pending_ingest", &pending_ingest),
    ] {
        if health.exists && !health.valid_json {
            warnings.push(format!("{label} file is present but not valid JSON"));
        }
    }

    DoctorReport {
        cwd,
        scope_hash: hash,
        temp_dir: temp_dir.to_string_lossy().to_string(),
        temp_dir_writable,
        hyphae_available,
        rhizome_available,
        session_live,
        session_state,
        outcomes,
        volva_hook_events,
        pending_exports,
        pending_ingest,
        warnings,
    }
}

fn print_file_health(label: &str, health: &FileHealth) {
    println!("{label}_path={}", health.path);
    println!("{label}_exists={}", health.exists);
    println!("{label}_valid_json={}", health.valid_json);
}

fn normalized_cwd(cwd: Option<&str>) -> String {
    cwd.filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| env::temp_dir().to_string_lossy().to_string())
}

impl From<SessionState> for SessionStatus {
    fn from(session: SessionState) -> Self {
        Self {
            session_id: session.session_id,
            project: session.project,
            project_root: session.project_root,
            worktree_id: session.worktree_id,
            started_at: session.started_at,
        }
    }
}

fn session_state_path(hash: &str) -> PathBuf {
    temp_state_path("session", hash, "json")
}

fn outcomes_path(hash: &str) -> PathBuf {
    temp_state_path("outcomes", hash, "json")
}

fn volva_hook_events_path(hash: &str) -> PathBuf {
    temp_state_path("volva-hook-events", hash, "json")
}

fn pending_exports_path(hash: &str) -> PathBuf {
    temp_state_path("pending-exports", hash, "json")
}

fn pending_ingest_path(hash: &str) -> PathBuf {
    temp_state_path("pending-ingest", hash, "json")
}

fn json_vec_len(path: &Path) -> usize {
    load_json_file::<Vec<serde_json::Value>>(path).map_or(0, |entries| entries.len())
}

fn inspect_json_file(path: &Path) -> FileHealth {
    let exists = path.exists();
    let valid_json = if exists {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
            .is_some()
    } else {
        true
    };

    FileHealth {
        path: path.to_string_lossy().to_string(),
        exists,
        valid_json,
    }
}

fn temp_dir_is_writable(temp_dir: &Path) -> bool {
    let probe = temp_dir.join(format!("cortina-doctor-{}.tmp", std::process::id()));
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{OutcomeEvent, OutcomeKind};
    use crate::utils::{current_timestamp_ms, update_json_file};

    fn test_cwd(name: &str) -> String {
        let dir = env::temp_dir().join(format!("cortina-status-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp cwd");
        dir.to_string_lossy().to_string()
    }

    #[test]
    fn collect_status_reports_scoped_counts() {
        let cwd = test_cwd("status");
        let hash = scope_hash(Some(&cwd));
        update_json_file::<Vec<OutcomeEvent>, _, _>(outcomes_path(&hash), |events| {
            events.push(OutcomeEvent::new(
                OutcomeKind::ValidationPassed,
                "cargo test passed",
            ));
        })
        .expect("write outcomes");
        update_json_file::<Vec<serde_json::Value>, _, _>(volva_hook_events_path(&hash), |events| {
            events.push(serde_json::json!({
                "phase": "session_start",
                "backend_kind": "official-cli",
                "cwd": cwd,
                "prompt_text": "status smoke",
                "prompt_summary": "status smoke"
            }));
        })
        .expect("write volva hook events");
        update_json_file::<Vec<String>, _, _>(pending_exports_path(&hash), |entries| {
            entries.extend(["src/a.rs".to_string(), "src/b.rs".to_string()]);
        })
        .expect("write exports");
        update_json_file::<Vec<String>, _, _>(pending_ingest_path(&hash), |entries| {
            entries.push("docs/a.md".to_string());
        })
        .expect("write ingest");
        save_json_file(
            session_state_path(&hash),
            &SessionState {
                session_id: "ses_demo".to_string(),
                project: "demo".to_string(),
                project_root: Some(cwd.clone()),
                worktree_id: Some("git:demo".to_string()),
                legacy_scope: None,
                started_at: current_timestamp_ms(),
            },
        )
        .expect("write session");

        let report = collect_status(Some(&cwd));
        assert_eq!(report.scope_hash, hash);
        assert_eq!(report.outcome_count, 1);
        assert_eq!(report.volva_hook_event_count, 1);
        assert_eq!(report.pending_export_count, 2);
        assert_eq!(report.pending_ingest_count, 1);
        assert_eq!(
            report.session.as_ref().map(|s| s.session_id.as_str()),
            Some("ses_demo")
        );
        assert_eq!(
            report
                .session
                .as_ref()
                .and_then(|session| session.project_root.as_deref()),
            Some(cwd.as_str())
        );
        assert_eq!(
            report
                .session
                .as_ref()
                .and_then(|session| session.worktree_id.as_deref()),
            Some("git:demo")
        );
    }

    #[test]
    fn collect_doctor_flags_invalid_json_state() {
        let cwd = test_cwd("doctor");
        let hash = scope_hash(Some(&cwd));
        fs::write(outcomes_path(&hash), "{not-json").expect("write invalid json");
        fs::write(volva_hook_events_path(&hash), "{not-json").expect("write invalid json");

        let report = collect_doctor(Some(&cwd));
        assert!(report.outcomes.exists);
        assert!(!report.outcomes.valid_json);
        assert!(report.volva_hook_events.exists);
        assert!(!report.volva_hook_events.valid_json);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("outcomes file"))
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("volva_hook_events file"))
        );
    }

    #[test]
    fn collect_status_json_hides_internal_legacy_scope_marker() {
        let cwd = test_cwd("status-legacy-scope");
        let hash = scope_hash(Some(&cwd));
        save_json_file(
            session_state_path(&hash),
            &SessionState {
                session_id: "ses_demo".to_string(),
                project: "demo".to_string(),
                project_root: Some(cwd.clone()),
                worktree_id: Some("git:demo".to_string()),
                legacy_scope: Some("legacy-scope".to_string()),
                started_at: current_timestamp_ms(),
            },
        )
        .expect("write session");

        let json = serde_json::to_value(collect_status(Some(&cwd))).expect("serialize status");
        let session = json
            .get("session")
            .and_then(serde_json::Value::as_object)
            .expect("session object");

        assert!(!session.contains_key("_legacy_scope"));
    }
}

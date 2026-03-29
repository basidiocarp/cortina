// ─────────────────────────────────────────────────────────────────────────
// Shared utilities for hook handlers
// ─────────────────────────────────────────────────────────────────────────

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::io::Seek as _;
use std::io::SeekFrom;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

const LOCK_STALE_MS: u64 = 30_000;
const LOCK_WAIT_ATTEMPTS: usize = 1_000;
const LOCK_WAIT_MS: u64 = 10;
const LOCK_HEARTBEAT_MS: u64 = 5_000;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

// ─────────────────────────────────────────────────────────────────────────
// Importance levels for stored content
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum Importance {
    #[allow(dead_code, reason = "Reserved for future use")]
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub project: String,
    #[serde(default)]
    pub started_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Success,
    Failure,
}

impl Importance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Check if a binary exists in PATH
pub fn command_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

pub fn scope_hash(cwd: Option<&str>) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let scope = normalize_scope_cwd(cwd);

    let mut hasher = DefaultHasher::new();
    scope.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn normalize_scope_cwd(cwd: Option<&str>) -> String {
    match cwd.filter(|value| !value.trim().is_empty()) {
        Some(path) => path.to_string(),
        None => env::current_dir().map_or_else(
            |_| env::temp_dir().to_string_lossy().to_string(),
            |p| p.to_string_lossy().to_string(),
        ),
    }
}

/// Resolve a temp state file path for Cortina tracking state.
pub fn temp_state_path(name: &str, hash: &str, extension: &str) -> PathBuf {
    env::temp_dir().join(format!("cortina-{name}-{hash}.{extension}"))
}

pub fn current_timestamp_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn session_state_path(hash: &str) -> PathBuf {
    temp_state_path("session", hash, "json")
}

pub fn project_name_for_cwd(cwd: Option<&str>) -> Option<String> {
    cwd.map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

/// Normalize a command for tracking (e.g., "cargo test --lib" -> "cargo test")
pub fn normalize_command(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.len() >= 2 {
        format!("{} {}", parts[0], parts[1])
    } else if !parts.is_empty() {
        parts[0].to_string()
    } else {
        cmd.to_string()
    }
}

/// Detect if output contains error patterns
pub fn has_error(output: &str, exit_code: Option<i32>) -> bool {
    // Check exit code
    if let Some(code) = exit_code {
        if code != 0 {
            return true;
        }
    }

    // Check for error patterns
    for re in error_patterns() {
        if re.is_match(output) {
            return true;
        }
    }

    false
}

fn error_patterns() -> &'static [Regex] {
    static ERROR_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    ERROR_PATTERNS.get_or_init(|| {
        [
            r"\berror[\s:[\]]",
            r"\bFAILED\b",
            r"\bpanicked\b",
            r"\bfailed\b",
            r"\bfatal[\s:]",
            r"\bcommand not found\b",
            r"\bsegmentation fault\b",
            r"\baborted\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Check if a file path is a document file (markdown, config, etc.)
pub fn is_document_file(path: &str) -> bool {
    let extensions = [
        ".md", ".txt", ".rst", ".adoc", // Documentation
        ".json", ".yaml", ".yml", ".toml", // Config/Data
        ".html", ".css", // Web
        ".env", ".cfg", ".ini", // Environment/Config
        ".sh", ".sql", // Scripts/Queries
    ];

    let path_lower = path.to_lowercase();
    extensions.iter().any(|ext| path_lower.ends_with(ext))
}

/// Check if a command is a build command
pub fn is_build_command(cmd: &str) -> bool {
    for re in build_patterns() {
        if re.is_match(cmd) {
            return true;
        }
    }

    false
}

/// Check if a command is a test command.
pub fn is_test_command(cmd: &str) -> bool {
    for re in test_patterns() {
        if re.is_match(cmd) {
            return true;
        }
    }

    false
}

fn build_patterns() -> &'static [Regex] {
    static BUILD_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    BUILD_PATTERNS.get_or_init(|| {
        [
            r"\bcargo\s+(build|check)\b",
            r"\bnpm\s+run\s+build\b",
            r"\byarn\s+build\b",
            r"\bpnpm\s+build\b",
            r"\bbun\s+build\b",
            r"\btsc\b",
            r"\bnext\s+build\b",
            r"\bmake\b",
            r"\bgo\s+build\b",
            r"\bgradlew\s+build\b",
            r"\bmvn\s+clean\s+package\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

fn test_patterns() -> &'static [Regex] {
    static TEST_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    TEST_PATTERNS.get_or_init(|| {
        [
            r"\bcargo\s+test\b",
            r"\bnpm\s+test\b",
            r"\bnpm\s+run\s+test\b",
            r"\byarn\s+test\b",
            r"\byarn\s+run\s+test\b",
            r"\bpnpm\s+test\b",
            r"\bpnpm\s+run\s+test\b",
            r"\bbun\s+test\b",
            r"\bpytest\b",
            r"\bgo\s+test\b",
            r"\bvitest\b",
            r"\bjest\b",
            r"\bplaywright\s+test\b",
            r"\bgradlew\s+test\b",
            r"\bmvn\s+test\b",
            r"\bmake\s+test\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Check if a command is a significant (worth tracking) command
pub fn is_significant_command(cmd: &str) -> bool {
    for re in significant_patterns() {
        if re.is_match(cmd) {
            return true;
        }
    }

    false
}

/// Return a positive Hyphae feedback signal for successful build/test commands.
pub fn successful_validation_feedback(
    cmd: &str,
    exit_code: Option<i32>,
) -> Option<(&'static str, i64, &'static str)> {
    if exit_code != Some(0) {
        return None;
    }

    if is_test_command(cmd) {
        Some(("test_passed", 1, "cortina.post_tool_use.test"))
    } else if is_build_command(cmd) {
        Some(("build_passed", 1, "cortina.post_tool_use.build"))
    } else {
        None
    }
}

fn significant_patterns() -> &'static [Regex] {
    static SIG_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

    SIG_PATTERNS.get_or_init(|| {
        [
            r"\bcargo\b",
            r"\bnpm\b",
            r"\byarn\b",
            r"\bpnpm\b",
            r"\bbun\b",
            r"\bgit\s+push\b",
            r"\bdocker\b",
            r"\bpytest\b",
            r"\bmake\b",
            r"\bgo\s+(build|test|run|vet)\b",
            r"\brustc\b",
            r"\bgcc\b",
            r"\bg\+\+\b",
            r"\bjavac\b",
            r"\bmvn\b",
            r"\bgradle\b",
            r"\bvitest\b",
            r"\bjest\b",
            r"\bplaywright\b",
            r"\btsc\b",
            r"\bpython\b",
            r"\bruby\b",
            r"\bswift\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Load JSON from a temp file
pub fn load_json_file<T: serde::de::DeserializeOwned>(path: impl AsRef<Path>) -> Option<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
}

/// Save JSON to a temp file
pub fn save_json_file<T: serde::Serialize>(path: impl AsRef<Path>, data: &T) -> Result<()> {
    let path = path.as_ref();
    let json = serde_json::to_string_pretty(data)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state");
    let temp_path = path.with_file_name(format!(
        ".{file_name}.{}.{}.{}.tmp",
        std::process::id(),
        current_timestamp_ms(),
        TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let mut temp_file = fs::File::create(&temp_path)?;
    temp_file.write_all(json.as_bytes())?;
    temp_file.sync_all()?;
    drop(temp_file);

    replace_file_atomic(&temp_path, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp_path);
    })?;
    Ok(())
}

pub fn update_json_file<T, F, R>(path: impl AsRef<Path>, mutator: F) -> Result<R>
where
    T: serde::de::DeserializeOwned + serde::Serialize + Default,
    F: FnOnce(&mut T) -> R,
{
    let path = path.as_ref();
    with_file_lock(path, || {
        let mut data = load_json_file(path).unwrap_or_default();
        let result = mutator(&mut data);
        save_json_file(path, &data)?;
        Ok(result)
    })
}

pub fn remove_file_with_lock(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    with_file_lock(path, || {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    })
}

fn with_file_lock<R>(path: &Path, operation: impl FnOnce() -> Result<R>) -> Result<R> {
    let lock_path = lock_path_for(path);
    with_lock_path(&lock_path, false, operation)
}

fn with_lock_path<R>(
    lock_path: &Path,
    heartbeat: bool,
    operation: impl FnOnce() -> Result<R>,
) -> Result<R> {
    let _guard = FileLockGuard::acquire(lock_path, heartbeat)?;
    operation()
}

fn lock_path_for(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}.lock",
        path.extension().and_then(|value| value.to_str()).unwrap_or("lock")
    ))
}

fn session_operation_lock_path(hash: &str) -> PathBuf {
    temp_state_path("session-op", hash, "json.lock")
}

fn with_session_operation_lock<R>(hash: &str, operation: impl FnOnce() -> Result<R>) -> Result<R> {
    with_lock_path(&session_operation_lock_path(hash), true, operation)
}

struct FileLockGuard {
    lock_path: PathBuf,
    token: String,
    #[allow(dead_code, reason = "Keeps the owned lock file handle alive until drop")]
    file: Option<std::sync::Arc<Mutex<fs::File>>>,
    stop_heartbeat: std::sync::Arc<AtomicBool>,
    heartbeat: Option<JoinHandle<()>>,
}

impl FileLockGuard {
    fn acquire(lock_path: &Path, heartbeat: bool) -> Result<Self> {
        for _ in 0..LOCK_WAIT_ATTEMPTS {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(lock_path)
            {
                Ok(file) => {
                    let token = format!(
                        "{}-{}-{}",
                        std::process::id(),
                        current_timestamp_ms(),
                        TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
                    );
                    let file = std::sync::Arc::new(Mutex::new(file));
                    let write_result = {
                        let mut guard = file.lock().expect("lock file mutex poisoned");
                        write_lock_metadata(&mut guard, &token)
                    };
                    if let Err(error) = write_result {
                        drop(file);
                        let _ = fs::remove_file(lock_path);
                        return Err(error);
                    }
                    let stop_heartbeat = std::sync::Arc::new(AtomicBool::new(false));
                    let heartbeat = heartbeat.then(|| {
                        let heartbeat_stop = std::sync::Arc::clone(&stop_heartbeat);
                        let heartbeat_token = token.clone();
                        let heartbeat_path = lock_path.to_path_buf();
                        let heartbeat_file = std::sync::Arc::clone(&file);
                        thread::spawn(move || {
                            while !heartbeat_stop.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(LOCK_HEARTBEAT_MS));
                                if heartbeat_stop.load(Ordering::Relaxed) {
                                    break;
                                }
                                if !refresh_lock_if_owned(
                                    &heartbeat_path,
                                    &heartbeat_token,
                                    &heartbeat_file,
                                ) {
                                    break;
                                }
                            }
                        })
                    });
                    return Ok(Self {
                        lock_path: lock_path.to_path_buf(),
                        token,
                        file: Some(file),
                        stop_heartbeat,
                        heartbeat,
                    });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    if lock_is_stale(lock_path) {
                        let _ = fs::remove_file(lock_path);
                        continue;
                    }
                    thread::sleep(Duration::from_millis(LOCK_WAIT_MS));
                }
                Err(error) => return Err(error.into()),
            }
        }

        Err(anyhow::anyhow!(
            "timed out waiting for file lock: {}",
            lock_path.display()
        ))
    }
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        self.stop_heartbeat.store(true, Ordering::Relaxed);
        if let Some(heartbeat) = self.heartbeat.take() {
            let _ = heartbeat.join();
        }
        let _ = self.file.take();
        if lock_token_matches(&self.lock_path, &self.token) {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

fn write_lock_metadata(file: &mut fs::File, token: &str) -> Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(format!("{token} {}\n", current_timestamp_ms()).as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn refresh_lock_if_owned(
    lock_path: &Path,
    token: &str,
    file: &std::sync::Arc<Mutex<fs::File>>,
) -> bool {
    if !lock_token_matches(lock_path, token) {
        return false;
    }

    write_lock_metadata(&mut file.lock().expect("lock file mutex poisoned"), token).is_ok()
}

fn lock_token_matches(lock_path: &Path, token: &str) -> bool {
    read_lock_metadata(lock_path)
        .map(|(current_token, _)| current_token == token)
        .unwrap_or(false)
}

fn read_lock_metadata(lock_path: &Path) -> Result<(String, u64)> {
    let contents = fs::read_to_string(lock_path)?;
    let mut parts = contents.split_whitespace();
    let token = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing lock token"))?
        .to_string();
    let timestamp = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing lock timestamp"))?
        .parse::<u64>()?;
    Ok((token, timestamp))
}

fn lock_is_stale(lock_path: &Path) -> bool {
    if let Ok((_, timestamp)) = read_lock_metadata(lock_path)
    {
        return current_timestamp_ms().saturating_sub(timestamp) > LOCK_STALE_MS;
    }

    lock_path
        .metadata()
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .is_some_and(|elapsed_ms| elapsed_ms > LOCK_STALE_MS)
}

#[cfg(not(windows))]
fn replace_file_atomic(temp_path: &Path, path: &Path) -> Result<()> {
    fs::rename(temp_path, path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file_atomic(temp_path: &Path, path: &Path) -> Result<()> {
    let mut from: Vec<u16> = temp_path.as_os_str().encode_wide().collect();
    from.push(0);
    let mut to: Vec<u16> = path.as_os_str().encode_wide().collect();
    to.push(0);

    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}

pub fn load_session_state(hash: &str) -> Option<SessionState> {
    load_json_file(session_state_path(hash))
}

#[cfg(test)]
pub fn clear_session_state(hash: &str) {
    let _ = remove_file_with_lock(session_state_path(hash));
}

pub fn ensure_scoped_hyphae_session(cwd: Option<&str>, task: Option<&str>) -> Option<SessionState> {
    if !command_exists("hyphae") {
        return None;
    }

    let project = project_name_for_cwd(cwd)?;
    ensure_hyphae_session_with_hash(&scope_hash(cwd), &project, task, Command::output)
}

fn ensure_hyphae_session_with_hash<F>(
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

            let mut cmd = Command::new("hyphae");
            cmd.args(["session", "start", "--project", project, "--scope", hash]);
            if let Some(task_desc) = task.filter(|value| !value.trim().is_empty()) {
                cmd.args(["--task", task_desc]);
            }

            let Ok(output) = run_command(&mut cmd) else {
                return Ok(None);
            };
            if !output.status.success() {
                return Ok(None);
            }

            let session_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if session_id.is_empty() {
                return Ok(None);
            }

            let new_state = SessionState {
                session_id,
                project: project.to_string(),
                started_at: current_timestamp_ms(),
            };

            return with_file_lock(&path, || {
                match load_json_file::<SessionState>(&path) {
                    Some(current) if current.session_id != existing.session_id => Ok(Some(current)),
                    _ => {
                        save_json_file(&path, &new_state)?;
                        Ok(Some(new_state.clone()))
                    }
                }
            });
        }

        let mut cmd = Command::new("hyphae");
        cmd.args(["session", "start", "--project", project, "--scope", hash]);
        if let Some(task_desc) = task.filter(|value| !value.trim().is_empty()) {
            cmd.args(["--task", task_desc]);
        }

        let Ok(output) = run_command(&mut cmd) else {
            return Ok(None);
        };
        if !output.status.success() {
            return Ok(None);
        }

        let session_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if session_id.is_empty() {
            return Ok(None);
        }

        let new_state = SessionState {
            session_id,
            project: project.to_string(),
            started_at: current_timestamp_ms(),
        };

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
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == hash)
        && parsed
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
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

fn end_hyphae_session_with<F>(
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

pub fn session_outcome_feedback(
    outcome_text: &str,
    saw_failures: bool,
) -> (&'static str, i64, &'static str, SessionOutcome) {
    let normalized = outcome_text.to_ascii_lowercase();
    let failed = saw_failures
        || normalized.contains("failed")
        || normalized.contains("failure")
        || normalized.contains("panic")
        || normalized.contains("aborted");

    if failed {
        (
            "session_failure",
            -1,
            "cortina.stop.session_failure",
            SessionOutcome::Failure,
        )
    } else {
        (
            "session_success",
            1,
            "cortina.stop.session_success",
            SessionOutcome::Success,
        )
    }
}

/// Store content in Hyphae (fire and forget)
pub fn store_in_hyphae(topic: &str, content: &str, importance: Importance, project: Option<&str>) {
    if !command_exists("hyphae") {
        return;
    }

    let mut cmd = Command::new("hyphae");
    cmd.args(["store", "--topic", topic])
        .args(["--content", content])
        .args(["--importance", importance.as_str()])
        .args(["--keywords", "cortina,hook"]);

    if let Some(proj) = project {
        cmd.args(["-P", proj]);
    }

    // Fire and forget — spawn without waiting
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn spawn_async_checked(cmd: &str, args: &[&str]) -> bool {
    let mut command = Command::new(cmd);
    for arg in args {
        command.arg(arg);
    }

    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;
    use std::process::ExitStatus;

    fn output_with_status(code: i32, stdout: &str) -> Output {
        Output {
            status: exit_status_from_code(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[cfg(unix)]
    fn exit_status_from_code(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit_status_from_code(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    // ─────────────────────────────────────────────────────────────────────
    // has_error tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_has_error_with_non_zero_exit_code() {
        assert!(has_error("", Some(1)));
        assert!(has_error("anything", Some(127)));
        assert!(has_error("", Some(-1)));
    }

    #[test]
    fn test_has_error_with_zero_exit_code_and_no_error_patterns() {
        assert!(!has_error("Success", Some(0)));
        assert!(!has_error("completed successfully", Some(0)));
    }

    #[test]
    fn test_has_error_with_error_pattern_in_output() {
        // Pattern: \bfailed\b
        assert!(
            has_error("Command failed", Some(0)),
            "Should detect 'failed'"
        );
        // Pattern: \bFAILED\b
        assert!(
            has_error("FAILED: test suite", Some(0)),
            "Should detect 'FAILED'"
        );
        // Pattern: \bpanicked\b
        assert!(
            has_error("thread panicked", Some(0)),
            "Should detect 'panicked'"
        );
    }

    #[test]
    fn test_has_error_with_none_exit_code_and_no_patterns() {
        assert!(!has_error("Output without errors", None));
    }

    #[test]
    fn test_has_error_with_none_exit_code_but_error_pattern() {
        // Pattern: \bcommand not found\b
        assert!(has_error("command not found", None));
        // Pattern: \bsegmentation fault\b
        assert!(has_error("segmentation fault in malloc", None));
    }

    // ─────────────────────────────────────────────────────────────────────
    // is_build_command tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_is_build_command_cargo() {
        assert!(is_build_command("cargo build"));
        assert!(is_build_command("cargo build --release"));
        assert!(is_build_command("cargo check"));
    }

    #[test]
    fn test_is_build_command_npm_and_tsc() {
        assert!(is_build_command("npm run build"));
        assert!(is_build_command("tsc"));
        assert!(is_build_command("make"));
    }

    #[test]
    fn test_is_build_command_non_build() {
        assert!(!is_build_command("ls -la"));
        assert!(!is_build_command("git status"));
        assert!(!is_build_command("echo hello"));
    }

    #[test]
    fn test_is_test_command() {
        assert!(is_test_command("cargo test"));
        assert!(is_test_command("npm run test"));
        assert!(is_test_command("make test"));
        assert!(!is_test_command("cargo build"));
    }

    #[test]
    fn test_successful_validation_feedback_prefers_test_commands() {
        assert_eq!(
            successful_validation_feedback("make test", Some(0)),
            Some(("test_passed", 1, "cortina.post_tool_use.test"))
        );
        assert_eq!(
            successful_validation_feedback("cargo build", Some(0)),
            Some(("build_passed", 1, "cortina.post_tool_use.build"))
        );
        assert_eq!(successful_validation_feedback("cargo test", Some(1)), None);
        assert_eq!(successful_validation_feedback("git status", Some(0)), None);
    }

    // ─────────────────────────────────────────────────────────────────────
    // normalize_command tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_normalize_command_multi_word() {
        assert_eq!(normalize_command("cargo build --release"), "cargo build");
        assert_eq!(
            normalize_command("cargo test --lib -- --nocapture"),
            "cargo test"
        );
    }

    #[test]
    fn test_normalize_command_single_word() {
        assert_eq!(normalize_command("ls"), "ls");
        assert_eq!(normalize_command("git"), "git");
    }

    #[test]
    fn test_normalize_command_empty() {
        assert_eq!(normalize_command(""), "");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Importance::as_str tests
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn test_importance_as_str() {
        assert_eq!(Importance::Low.as_str(), "low");
        assert_eq!(Importance::Medium.as_str(), "medium");
        assert_eq!(Importance::High.as_str(), "high");
    }

    #[test]
    fn test_temp_state_path_uses_system_temp_dir() {
        let path = temp_state_path("errors", "abc123", "json");
        assert!(path.starts_with(env::temp_dir()));
        assert!(path.ends_with("cortina-errors-abc123.json"));
    }

    #[test]
    fn test_scope_hash_uses_explicit_cwd_when_present() {
        assert_eq!(scope_hash(Some("/tmp/demo")), scope_hash(Some("/tmp/demo")));
        assert_ne!(
            scope_hash(Some("/tmp/demo-a")),
            scope_hash(Some("/tmp/demo-b"))
        );
    }

    #[test]
    fn test_project_name_for_cwd_uses_explicit_path() {
        assert_eq!(
            project_name_for_cwd(Some("/tmp/demo-project")).as_deref(),
            Some("demo-project")
        );
    }

    #[test]
    fn test_ensure_hyphae_session_with_runner_leaves_state_empty_on_spawn_failure() {
        let hash = "ensure-spawn-failure";
        clear_session_state(hash);

        let state = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |_cmd| {
            Err(std::io::Error::other("spawn failed"))
        });

        assert!(state.is_none());
        assert!(load_session_state(hash).is_none());
    }

    #[test]
    fn test_ensure_hyphae_session_with_runner_reuses_active_cached_state() {
        let hash = "ensure-active-state";
        clear_session_state(hash);
        let state = SessionState {
            session_id: "ses_active".to_string(),
            project: "demo-project".to_string(),
            started_at: 1,
        };
        save_json_file(session_state_path(hash), &state).unwrap();

        let mut status_calls = 0;
        let mut start_calls = 0;
        let result = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
            let args: Vec<String> = cmd
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();

            match args.as_slice() {
                ["session", "status", "--id", "ses_active"] => {
                    status_calls += 1;
                    Ok(output_with_status(
                        0,
                        r#"{"session_id":"ses_active","project":"demo-project","scope":"ensure-active-state","status":"active","active":true}"#,
                    ))
                }
                [
                    "session",
                    "start",
                    "--project",
                    "demo-project",
                    "--scope",
                    "ensure-active-state",
                    "--task",
                    "task",
                ] => {
                    start_calls += 1;
                    Ok(output_with_status(0, "ses_new"))
                }
                _ => panic!("unexpected hyphae command args: {args:?}"),
            }
        });

        assert_eq!(result.as_ref(), Some(&state));
        assert_eq!(status_calls, 1);
        assert_eq!(start_calls, 0);
        assert_eq!(load_session_state(hash).as_ref(), Some(&state));
        clear_session_state(hash);
    }

    #[test]
    fn test_ensure_hyphae_session_with_runner_discards_stale_cached_state() {
        let hash = "ensure-stale-state";
        clear_session_state(hash);
        let stale = SessionState {
            session_id: "ses_stale".to_string(),
            project: "demo-project".to_string(),
            started_at: 1,
        };
        save_json_file(session_state_path(hash), &stale).unwrap();

        let mut status_calls = 0;
        let mut start_calls = 0;
        let result = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
            let args: Vec<String> = cmd
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();

            match args.as_slice() {
                ["session", "status", "--id", "ses_stale"] => {
                    status_calls += 1;
                    Ok(output_with_status(
                        0,
                        r#"{"session_id":"ses_stale","project":"demo-project","scope":"ensure-stale-state","status":"completed","active":false}"#,
                    ))
                }
                [
                    "session",
                    "start",
                    "--project",
                    "demo-project",
                    "--scope",
                    "ensure-stale-state",
                    "--task",
                    "task",
                ] => {
                    start_calls += 1;
                    Ok(output_with_status(0, "ses_fresh"))
                }
                _ => panic!("unexpected hyphae command args: {args:?}"),
            }
        });

        assert_eq!(
            result.as_ref().map(|session| session.session_id.as_str()),
            Some("ses_fresh")
        );
        assert_eq!(status_calls, 1);
        assert_eq!(start_calls, 1);
        assert_eq!(
            load_session_state(hash)
                .as_ref()
                .map(|session| session.session_id.as_str()),
            Some("ses_fresh")
        );
        clear_session_state(hash);
    }

    #[test]
    fn test_ensure_hyphae_session_with_runner_ignores_other_scoped_sessions() {
        let hash = "ensure-scope-a";
        clear_session_state(hash);
        let cached = SessionState {
            session_id: "ses_scope_a".to_string(),
            project: "demo-project".to_string(),
            started_at: 1,
        };
        save_json_file(session_state_path(hash), &cached).unwrap();

        let mut status_calls = 0;
        let mut start_calls = 0;
        let result = ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
            let args: Vec<String> = cmd
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();

            match args.as_slice() {
                ["session", "status", "--id", "ses_scope_a"] => {
                    status_calls += 1;
                    Ok(output_with_status(
                        0,
                        r#"{"session_id":"ses_scope_a","project":"demo-project","scope":"ensure-scope-b","status":"active","active":true}"#,
                    ))
                }
                [
                    "session",
                    "start",
                    "--project",
                    "demo-project",
                    "--scope",
                    "ensure-scope-a",
                    "--task",
                    "task",
                ] => {
                    start_calls += 1;
                    Ok(output_with_status(0, "ses_scope_a_fresh"))
                }
                _ => panic!("unexpected hyphae command args: {args:?}"),
            }
        });

        assert_eq!(
            result.as_ref().map(|session| session.session_id.as_str()),
            Some("ses_scope_a_fresh")
        );
        assert_eq!(status_calls, 1);
        assert_eq!(start_calls, 1);
        clear_session_state(hash);
    }

    #[test]
    fn test_ensure_hyphae_session_with_runner_serializes_concurrent_starts() {
        let hash = "ensure-concurrent-start";
        clear_session_state(hash);

        let start_calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();

        for _ in 0..2 {
            let start_calls = Arc::clone(&start_calls);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                ensure_hyphae_session_with_hash(hash, "demo-project", Some("task"), |cmd| {
                    let args: Vec<String> = cmd
                        .get_args()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect();
                    let args = args.iter().map(String::as_str).collect::<Vec<_>>();

                    match args.as_slice() {
                        ["session", "start", "--project", "demo-project", "--scope", "ensure-concurrent-start", "--task", "task"] => {
                            let call = start_calls.fetch_add(1, Ordering::SeqCst) + 1;
                            thread::sleep(Duration::from_millis(50));
                            Ok(output_with_status(0, &format!("ses_{call}")))
                        }
                        ["session", "status", "--id", "ses_1"] => Ok(output_with_status(
                            0,
                            r#"{"session_id":"ses_1","project":"demo-project","scope":"ensure-concurrent-start","status":"active","active":true}"#,
                        )),
                        unexpected => panic!("unexpected hyphae command args: {unexpected:?}"),
                    }
                })
                .expect("session should be created")
            }));
        }

        let first = handles.remove(0).join().expect("thread should succeed");
        let second = handles.remove(0).join().expect("thread should succeed");

        assert_eq!(start_calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.session_id, "ses_1");
        assert_eq!(second.session_id, "ses_1");
        clear_session_state(hash);
    }

    #[test]
    fn test_end_hyphae_session_with_missing_state_returns_false() {
        let hash = "end-missing-state";
        clear_session_state(hash);

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Ok(output_with_status(0, ""))
        });

        assert!(result.is_none());
    }

    #[test]
    fn test_end_hyphae_session_with_spawn_failure_keeps_cached_state() {
        let hash = "end-spawn-failure";
        clear_session_state(hash);
        let state = SessionState {
            session_id: "ses_demo".to_string(),
            project: "demo-project".to_string(),
            started_at: 1,
        };
        save_json_file(session_state_path(hash), &state).unwrap();

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Err(std::io::Error::other("spawn failed"))
        });

        assert!(result.is_none());
        assert_eq!(load_session_state(hash).as_ref(), Some(&state));
        clear_session_state(hash);
    }

    #[test]
    fn test_end_hyphae_session_with_non_zero_exit_keeps_cached_state() {
        let hash = "end-non-zero";
        clear_session_state(hash);
        let state = SessionState {
            session_id: "ses_demo".to_string(),
            project: "demo-project".to_string(),
            started_at: 1,
        };
        save_json_file(session_state_path(hash), &state).unwrap();

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Ok(output_with_status(1, "failed"))
        });

        assert!(result.is_none());
        assert_eq!(load_session_state(hash).as_ref(), Some(&state));
        clear_session_state(hash);
    }

    #[test]
    fn test_end_hyphae_session_with_success_clears_cached_state() {
        let hash = "end-success";
        clear_session_state(hash);
        let state = SessionState {
            session_id: "ses_demo".to_string(),
            project: "demo-project".to_string(),
            started_at: 1,
        };
        save_json_file(session_state_path(hash), &state).unwrap();

        let result = end_hyphae_session_with(hash, Some("summary"), &[], 0, |_cmd| {
            Ok(output_with_status(0, "ok"))
        });

        assert_eq!(result.as_ref(), Some(&state));
        assert!(load_session_state(hash).is_none());
    }

    #[test]
    fn test_session_outcome_feedback_classifies_failure_keywords() {
        assert_eq!(
            session_outcome_feedback("Build failed after retries", false).3,
            SessionOutcome::Failure
        );
        assert_eq!(
            session_outcome_feedback("Work completed successfully", false).3,
            SessionOutcome::Success
        );
        assert_eq!(
            session_outcome_feedback("Work completed", true).3,
            SessionOutcome::Failure
        );
        assert_eq!(
            session_outcome_feedback("Improved error handling and validation", false).3,
            SessionOutcome::Success
        );
    }

    #[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    struct CounterState {
        value: usize,
    }

    #[test]
    fn test_update_json_file_serializes_concurrent_mutations() {
        let path = temp_state_path("counter", "concurrent-update", "json");
        let _ = fs::remove_file(&path);

        let workers = 16;
        let barrier = Arc::new(Barrier::new(workers));
        let mut handles = Vec::new();

        for _ in 0..workers {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                update_json_file::<CounterState, _, _>(&path, |state| {
                    state.value += 1;
                })
                .expect("counter update should succeed");
            }));
        }

        for handle in handles {
            handle.join().expect("thread should finish cleanly");
        }

        let state: CounterState = load_json_file(&path).expect("counter state should exist");
        assert_eq!(state.value, workers);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_update_json_file_recovers_stale_lock() {
        let path = temp_state_path("counter", "stale-lock", "json");
        let lock_path = lock_path_for(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&lock_path);
        fs::write(
            &lock_path,
            format!(
                "stale-owner {}\n",
                current_timestamp_ms().saturating_sub(LOCK_STALE_MS + 1_000)
            ),
        )
        .unwrap();

        update_json_file::<CounterState, _, _>(&path, |state| {
            state.value = 7;
        })
        .expect("stale lock should be recovered");

        let state: CounterState = load_json_file(&path).expect("counter state should exist");
        assert_eq!(state.value, 7);

        let _ = fs::remove_file(path);
        let _ = fs::remove_file(lock_path);
    }
}

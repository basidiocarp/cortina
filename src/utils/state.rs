use anyhow::Result;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::io::Seek as _;
use std::io::SeekFrom;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

pub fn scope_hash(cwd: Option<&str>) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let scope = normalize_scope_cwd(cwd);

    let mut hasher = DefaultHasher::new();
    scope.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

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

pub fn load_json_file<T: serde::de::DeserializeOwned>(path: impl AsRef<Path>) -> Option<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
}

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
    with_file_lock(path, || match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    })
}

pub(super) fn with_file_lock<R>(path: &Path, operation: impl FnOnce() -> Result<R>) -> Result<R> {
    let lock_path = lock_path_for(path);
    with_lock_path(&lock_path, false, operation)
}

pub(super) fn with_lock_path<R>(
    lock_path: &Path,
    heartbeat: bool,
    operation: impl FnOnce() -> Result<R>,
) -> Result<R> {
    let _guard = FileLockGuard::acquire(lock_path, heartbeat)?;
    operation()
}

#[cfg(test)]
pub(super) fn lock_path_for(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}.lock",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("lock")
    ))
}

#[cfg(not(test))]
fn lock_path_for(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}.lock",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("lock")
    ))
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

struct FileLockGuard {
    lock_path: PathBuf,
    token: String,
    #[allow(
        dead_code,
        reason = "Keeps the owned lock file handle alive until drop"
    )]
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
    if let Ok((_, timestamp)) = read_lock_metadata(lock_path) {
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

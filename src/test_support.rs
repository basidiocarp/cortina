use std::sync::{Mutex, MutexGuard, OnceLock};

static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn test_lock() -> MutexGuard<'static, ()> {
    TEST_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("test lock should not be poisoned")
}

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static CURRENT_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static INITIAL_CWD: OnceLock<PathBuf> = OnceLock::new();

/// `std::env::set_current_dir` is not thread safe, it is disallowed via clippy, use this instead
pub(crate) struct SetCurrentDirTempSafe<'a> {
    tempdir: tempfile::TempDir,
    _lock: std::sync::MutexGuard<'a, ()>,
}

impl<'a> SetCurrentDirTempSafe<'a> {
    pub(crate) fn new() -> Self {
        // Recover from poisoned mutex - if a test panicked while holding the lock,
        // we can still safely proceed since we're just using it for synchronization
        let lock = CURRENT_DIR_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        INITIAL_CWD.get_or_init(|| std::env::current_dir().unwrap());
        let tempdir = tempfile::tempdir().unwrap();
        #[allow(clippy::disallowed_methods)]
        std::env::set_current_dir(tempdir.path()).unwrap();
        Self {
            tempdir,
            _lock: lock,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.tempdir.path()
    }
}

impl<'a> Drop for SetCurrentDirTempSafe<'a> {
    fn drop(&mut self) {
        if let Some(initial_cwd) = INITIAL_CWD.get() {
            #[allow(clippy::disallowed_methods)]
            std::env::set_current_dir(initial_cwd).unwrap();
        }
    }
}

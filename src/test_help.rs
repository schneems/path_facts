use std::path::{Path, PathBuf};

static CURRENT_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `std::env::set_current_dir` is not thread safe, it is disallowed via clippy, use this instead
pub(crate) struct SetCurrentDirTempSafe<'a> {
    cwd: PathBuf,
    tempdir: tempfile::TempDir,
    _lock: std::sync::MutexGuard<'a, ()>,
}

impl<'a> SetCurrentDirTempSafe<'a> {
    pub(crate) fn new() -> Self {
        let lock = CURRENT_DIR_LOCK.lock().unwrap();
        let tempdir = tempfile::tempdir().unwrap();
        let cwd = std::env::current_dir().unwrap();
        #[allow(clippy::disallowed_methods)]
        std::env::set_current_dir(tempdir.path()).unwrap();
        Self {
            cwd,
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
        #[allow(clippy::disallowed_methods)]
        std::env::set_current_dir(&self.cwd).unwrap();
    }
}

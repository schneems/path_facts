//! An absolute path may or may not exist on disk
//!
//! A property of absolute paths is that recursively retrieving their parent paths will eventually
//! lead to the root path.
//!
//! We also ensure other properties, such as ReadDir of the parent of a file should
//! return a `path()` that matches the file if it exists (i.e. it's the same representation).
//!
//! In order to turn a relative path into an absolute path, the current working directory
//! must be readable.
//!
use std::{
    fmt::{Display, Formatter},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbsPath(PathBuf);

impl AbsPath {
    pub(crate) fn new(path: impl AsRef<Path>) -> Result<Self, AbsPathError> {
        let path = path.as_ref();

        if path.as_os_str().is_empty() {
            return Err(AbsPathError::PathIsEmpty(path.to_owned()));
        }

        if path.is_relative() {
            let absolute = std::path::absolute(path)
                .map_err(|error| AbsPathError::CannotReadCWD(path.to_owned(), error))?;
            Ok(Self(absolute))
        } else {
            Ok(Self(path.to_owned()))
        }
    }

    /// Tries to read the current path as a directory
    ///
    /// The properties of `read_dir` state that the resulting paths returned from `DirEntry`
    /// match the original path appended with the filename of the entry. Because we know
    /// the directory path is absolute, we know the resulting paths are absolute.
    ///
    /// Further this gives us the properties that calling `AbsPath::parent().read_dir()` should
    /// return a vector of paths that contain the original path if the original file exists. i.e.
    /// the format is the same.
    ///
    /// Errors if path is not a directory or is not readable
    pub(crate) fn read_dir(&self) -> Result<Vec<AbsPath>, std::io::Error> {
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut entries: Vec<AbsPath> = std::fs::read_dir(&self.0)?
            .map(|entry| entry.map(|e| e.path()).map(AbsPath))
            .collect::<Result<Vec<AbsPath>, std::io::Error>>()?;

        // Sort by filename for deterministic test output only
        // In production, preserve the OS's native directory entry order
        #[cfg(test)]
        {
            entries.sort_by(|a, b| {
                let a_name = a.0.file_name().unwrap_or(a.0.as_os_str());
                let b_name = b.0.file_name().unwrap_or(b.0.as_os_str());
                a_name.cmp(b_name)
            });
        }

        Ok(entries)
    }

    // pub(crate) fn path_ok(self) -> Result<HappyPath, HappyPathError> {
    //     HappyPath::new(self)
    // }

    // Similar semantics to Path::parent, but returning a None here would guarantee self is the root path
    pub(crate) fn parent(&self) -> Option<Self> {
        let parent = self.0.parent()?;

        Some(AbsPath(parent.to_path_buf()))
    }
}

impl Display for AbsPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`", self.0.display())
    }
}

impl AsRef<Path> for AbsPath {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

/// Returns Err if `read_link` fails
/// Returns Ok(None) if the path is not a symlink or if `fs::symlink_metadata` fails
/// Otherwise returns Ok(Some(AbsPath)) with the target of the symlink
pub(crate) fn try_readlink(absolute: &AbsPath) -> Result<Option<AbsPath>, std::io::Error> {
    let path = absolute.as_ref();
    if path.is_symlink() {
        std::fs::read_link(path)
            .map(|target| {
                if target.is_relative() {
                    AbsPath(absolute.as_ref().join(target))
                } else {
                    AbsPath(target)
                }
            })
            .map(Some)
    } else {
        Ok(None)
    }
}

#[derive(Debug)]
pub(crate) enum AbsPathError {
    PathIsEmpty(PathBuf),
    CannotReadCWD(PathBuf, std::io::Error),
}

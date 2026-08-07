//! An absolute path that may or may not exist on disk
//!
//! Holding this type guarantees that the path is not empty and the program has permission to read CWD.
//!
//! A property of absolute paths is that recursively retrieving their parent paths will eventually
//! lead to the root path. The parent of an absolute path is also an absolute path [`AbsPath::parent`].
//!
//! If the held path is a readable directory, all children are also absolute paths [`AbsPath::read_dir`].
use std::{
    fmt::{Display, Formatter},
    path::{Path, PathBuf, StripPrefixError},
};

/// Guaranteed to be relative
#[derive(Debug)]
pub(crate) struct RelativePath(PathBuf);

impl RelativePath {
    pub(crate) fn new(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref();
        if path.is_relative() {
            Some(Self(path.to_owned()))
        } else {
            None
        }
    }
}

impl AsRef<Path> for RelativePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

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

    /// Returns `self` expressed relative to `base`.
    ///
    /// - The returned relative path may be empty (when `self == base`).
    /// - Purely lexical: `..` is NOT resolved, so `/root/a/../b` and `/root/b` differ.
    /// - Errors if `base` is not a lexical prefix of `self`.
    pub(crate) fn strip_prefix(&self, base: &AbsPath) -> Result<RelativePath, StripPrefixError> {
        let diff = self.as_ref().strip_prefix(base.as_ref())?;
        Ok(RelativePath::new(diff).expect("path with stripped prefix is guaranteed relative"))
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

    /// Similar semantics to [`Path::parent`], but returning a None here would guarantee self is the root path
    ///
    /// An `AbsPath` is not normalized so it may contain `..` and/or symlinks. This is a lexical operation.
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
/// Returns Ok(None) if the path is not a symlink or if [`std::fs::symlink_metadata`] fails
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

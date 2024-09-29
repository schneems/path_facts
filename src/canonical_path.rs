//! If a path can be canonicalized it exists on disk and any symlinks in it's
//! path can be resolved to entities on disk.
//!
//! It can still have other problems, such as being a file when it's
//! expected to be a directory or not having correct permissions, but
//! we can guarantee that all files involved exist.
use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

use crate::abs_path::AbsPath;

#[derive(Debug, Clone)]
pub(crate) struct CanonicalPath(PathBuf);

impl CanonicalPath {
    pub(crate) fn new(abs_path: &AbsPath) -> Result<Self, std::io::Error> {
        let canonical = abs_path.as_ref().canonicalize()?;
        Ok(CanonicalPath(canonical))
    }
}

impl AsRef<Path> for CanonicalPath {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl Display for CanonicalPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`", self.0.display())
    }
}

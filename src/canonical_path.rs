//! If a path can be canonicalized it exists on disk and any symlinks in it's
//! path can be resolved to entities on disk.
//!
//! It can still have other problems, such as being a file when it's
//! expected to be a directory or not having correct permissions, but
//! we can guarantee that all files involved exist.
//!
//! Built from a [`AbsPath`] so we know the program has access to CWD.
use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

use crate::abs_path::{AbsPath, RelativePath};

/// Represents a partially cannonicalized path
#[derive(Debug)]
pub(crate) struct PriorCanonicalPath {
    /// Parent or prior directory that can be canonicalized
    pub(crate) prior: CanonicalPath,
    /// Rest of the path that could not be canonicalized, may have un-normalized parts i.e. `..`
    pub(crate) rest: RelativePath,
}

/// Always contains a full or partial [`CanonicalPath`]
///
/// Any [`CanonicalPath`] exists, but we don't know other things about it without querying the
/// filesystem.
pub(crate) enum ExpandPath {
    /// Path exists, fully expanded
    Canonical(CanonicalPath),
    /// Part of a path exists, the full path either doesn't exist or we don't have permission or broken symlink somewhere
    Prior(PriorCanonicalPath),
}

#[derive(Debug)]
pub(crate) struct CannotCanonicalizeAnything {
    pub(crate) original: AbsPath,
    pub(crate) root: AbsPath,
    pub(crate) root_error: std::io::Error,
}

impl ExpandPath {
    pub(crate) fn new(abs_path: &AbsPath) -> Result<Self, CannotCanonicalizeAnything> {
        // Walk the original path, then each lexical ancestor up to root.
        let ancestors = std::iter::successors(Some(abs_path.clone()), AbsPath::parent);

        let mut last_failure = None;
        for path in ancestors.peekable() {
            match CanonicalPath::new(&path) {
                Ok(can_path) => {
                    let relative = abs_path
                        .strip_prefix(&path)
                        .expect("ancestor is a lexical prefix of the original path");

                    // Empty path when the two paths are the same
                    if relative.as_ref().as_os_str().is_empty() {
                        return Ok(ExpandPath::Canonical(can_path));
                    } else {
                        return Ok(ExpandPath::Prior(PriorCanonicalPath {
                            prior: can_path,
                            rest: relative,
                        }));
                    };
                }
                Err(io_error) => last_failure = Some((path, io_error)),
            }
        }

        let (root, root_error) =
            last_failure.expect("loop either returns or populates this value, loop guaranteed to have at least one value");
        Err(CannotCanonicalizeAnything {
            original: abs_path.clone(),
            root,
            root_error,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CanonicalPath(PathBuf);

impl CanonicalPath {
    pub(crate) fn new(abs_path: &AbsPath) -> Result<Self, std::io::Error> {
        let canonical = abs_path.as_ref().canonicalize()?;
        Ok(CanonicalPath(canonical))
    }

    /// Similar semantics to [`AbsPath::parent`], but we guarantee return value
    /// exists and is normalized i.e. any CanonicalPath that is lexically equal is guaranteed
    /// to represent the same path on disk (TOCTOU caveat).
    ///
    /// A None here would guarantee self is the root path
    pub(crate) fn parent(&self) -> Option<Self> {
        let parent = self.0.parent()?;

        Some(CanonicalPath(parent.to_path_buf()))
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

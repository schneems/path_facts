//! If a path can be canonicalized it exists on disk and any symlinks in it's
//! path can be resolved to entities on disk.
//!
//! It can still have other problems, such as being a file when it's
//! expected to be a directory or not having correct permissions, but
//! we can guarantee that all files involved exist.
//!
//! Built from a [`AbsPath`] so we know the program has access to CWD.
//! May have un-normalized parts i.e. `..`
use crate::abs_path::{AbsPath, RelativePath};
use std::{
    ffi::OsStr,
    fmt::Display,
    path::{Path, PathBuf},
};

/// Represents a partially cannonicalized path
#[derive(Debug, Clone)]
pub(crate) struct PartialCanonicalPath {
    /// Parent or prior directory that can be canonicalized
    #[allow(dead_code)] // Prove properties through type construction
    prior: CanonicalPath,
    /// Rest of the path that could not be canonicalized, may have un-normalized parts i.e. `..`
    #[allow(dead_code)] // Prove properties through type construction
    rest: RelativePath,

    /// Holds full path, allows us to impl `AsRef<Path>`
    full: AbsPath,
}

impl AsRef<Path> for PartialCanonicalPath {
    fn as_ref(&self) -> &Path {
        self.full.as_ref()
    }
}

impl AsRef<Path> for ExpandPath {
    fn as_ref(&self) -> &Path {
        match self {
            ExpandPath::Canonical(canonical_path) => canonical_path.as_ref(),
            ExpandPath::Partial(partial_canonical_path) => partial_canonical_path.as_ref(),
        }
    }
}

impl From<PartialCanonicalPath> for AbsPath {
    fn from(value: PartialCanonicalPath) -> Self {
        let PartialCanonicalPath {
            prior: _,
            rest: _,
            full,
        } = value;
        full
    }
}

/// Always contains a full or partial [`CanonicalPath`]
///
/// Any [`CanonicalPath`] exists, but we don't know other things about it without querying the
/// filesystem. A Partial variant may or may not exist, it may have un-normalized parts such as `..`
/// and `.` in it.
#[derive(Debug, Clone)]
pub(crate) enum ExpandPath {
    /// Path exists, fully expanded
    Canonical(CanonicalPath),
    /// Part of a path exists and can be expanded, the full path either doesn't exist or we don't
    /// have permission or there is a broken symlink somewhere. NOT lexically normalized, could
    /// contain `..` or `.` parts.
    Partial(PartialCanonicalPath),
}

impl From<ExpandPath> for AbsPath {
    fn from(value: ExpandPath) -> Self {
        match value {
            ExpandPath::Canonical(canonical_path) => canonical_path.into(),
            ExpandPath::Partial(partial_canonical_path) => partial_canonical_path.into(),
        }
    }
}

impl From<CanonicalPath> for ExpandPath {
    fn from(value: CanonicalPath) -> Self {
        ExpandPath::Canonical(value)
    }
}

impl From<PartialCanonicalPath> for ExpandPath {
    fn from(value: PartialCanonicalPath) -> Self {
        ExpandPath::Partial(value)
    }
}

#[derive(Debug)]
pub(crate) struct CannotCanonicalizeAnything {
    pub(crate) original: AbsPath,
    pub(crate) root: AbsPath,
    pub(crate) root_error: std::io::Error,
}

impl ExpandPath {
    #[allow(dead_code)]
    pub(crate) fn new(abs_path: &AbsPath) -> Result<Self, CannotCanonicalizeAnything> {
        // Walk the original path, then each lexical ancestor up to root.
        let ancestors = std::iter::successors(Some(abs_path.clone()), AbsPath::lex_parent);

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
                        return Ok(ExpandPath::Partial(PartialCanonicalPath {
                            prior: can_path,
                            rest: relative,
                            full: abs_path.clone(),
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

    #[allow(dead_code)]
    pub(crate) fn canonical(self) -> Result<CanonicalPath, PartialCanonicalPath> {
        match self {
            ExpandPath::Canonical(canonical_path) => Ok(canonical_path),
            ExpandPath::Partial(partial_canonical_path) => Err(partial_canonical_path),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn partial(self) -> Result<PartialCanonicalPath, CanonicalPath> {
        match self {
            ExpandPath::Canonical(canonical_path) => Err(canonical_path),
            ExpandPath::Partial(partial_canonical_path) => Ok(partial_canonical_path),
        }
    }
}

/// What [`CanonicalPath::entry`] found at a name inside a directory
#[derive(Debug)]
#[allow(dead_code)] // Only reached through `Trace::new`, not wired into output yet
pub(crate) enum Entry {
    /// Not a symlink, so the path is canonical and the metadata describes it directly
    Canonical(CanonicalPath, std::fs::Metadata),
    /// A symlink, which has to be followed before anything canonical can be said about it
    Symlink,
}

/// File exists, and is resolvable path, is fully normalized
///
/// - All symlinks resolve and are visible
/// - All directories involved are executable
///
/// Does not preserve behavior on all calls. Getting metadata from
/// `/a/b/file.txt/..` fails with NotADirectory on every unix, POSIX requires
/// ENOTDIR when a path prefix component is not a directory.
///
/// Canonicalizing it produces different results. Glibc enforces the same rule and errors, but a mac
/// returns `/a/b`. So on a mac this type gets built from a path the kernel refuses, and metadata on
/// the canonical form succeeds while metadata on the original still fails. A trailing `.` behaves the same way.
///
/// So using a `CanonicalPath` as a replacement for `Path` can yield subtle differences.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CanonicalPath(PathBuf);

impl From<CanonicalPath> for AbsPath {
    fn from(value: CanonicalPath) -> Self {
        AbsPath::new(value.0).unwrap()
    }
}

impl CanonicalPath {
    pub(crate) fn new(abs_path: &AbsPath) -> Result<Self, std::io::Error> {
        let canonical = abs_path.as_ref().canonicalize()?;
        Ok(CanonicalPath(canonical))
    }

    pub(crate) unsafe fn unchecked_join(&self, rest: &OsStr) -> CanonicalPath {
        CanonicalPath(self.as_ref().join(rest))
    }

    /// Looks up `name` in this directory
    ///
    /// Takes its own `symlink_metadata` rather than accepting one. Metadata handed in by a
    /// caller says nothing about `name`, it could describe any path at all, so it cannot
    /// stand as proof of anything about the value returned here.
    ///
    /// A non-symlink entry is canonical without a `realpath` call, meeting each part of the
    /// contract on [`CanonicalPath`]:
    ///
    /// - Exists, because `symlink_metadata` succeeded on it.
    /// - Fully normalized, because `self` holds no symlinks and no `.` or `..` parts, and
    ///   `lstat` shows `name` is not a symlink. A `Normal` component adds no dot parts.
    /// - Every directory involved is executable, because `self` already carried that and
    ///   the lookup of `name` inside it just succeeded, which requires it.
    ///
    /// Which leaves resolvable: `realpath` would walk `self` (symlink free, searchable)
    /// and then find `name` present and not a link, so it has nothing left to resolve.
    ///
    /// Carries the same TOCTOU caveat as everything else in this library: all of the above
    /// was true when the syscall ran.
    pub(crate) fn entry(&self, name: &OsStr) -> Result<Entry, std::io::Error> {
        let path = self.0.join(name);
        let lstat = std::fs::symlink_metadata(&path)?;

        if lstat.file_type().is_symlink() {
            Ok(Entry::Symlink)
        } else {
            Ok(Entry::Canonical(CanonicalPath(path), lstat))
        }
    }

    /// Returns list of all files that exist in the directory
    ///
    /// The properties of `read_dir` state that the resulting paths returned from `DirEntry`
    /// match the original path appended with the filename of the entry. Because we know
    /// the directory path is absolute, we know the resulting paths are absolute. However they
    /// aren't guaranteed to be resolvable.
    ///
    /// Errors if path is not a directory or is not readable
    #[allow(dead_code)]
    pub(crate) fn read_dir(&self) -> Result<Vec<ExpandPath>, std::io::Error> {
        let parent: AbsPath = self.clone().into();
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut entries = std::fs::read_dir(&self.0)?
            .map(|entry| {
                entry.map(|e| {
                    let prior = self.clone();
                    let rest = RelativePath::new(e.file_name())
                        .expect("read_dir to always return a relative path");
                    let full = parent.join_relative(&rest);
                    // Children are guaranteed to exist, but not guaranted to be resolvable (CanonicalPath)
                    // Instead of the extra filesystem calls represent the ambiguity as all entries being
                    // a partial canonical path.
                    ExpandPath::Partial(PartialCanonicalPath { prior, rest, full })
                })
            })
            .collect::<Result<Vec<ExpandPath>, std::io::Error>>()?;

        // Sort by filename for deterministic test output only
        // In production, preserve the OS's native directory entry order
        #[cfg(test)]
        {
            entries.sort_by(|a, b| {
                let a_name = a.as_ref().file_name().unwrap_or(a.as_ref().as_os_str());
                let b_name = b.as_ref().file_name().unwrap_or(b.as_ref().as_os_str());
                a_name.cmp(b_name)
            });
        }

        Ok(entries)
    }

    /// Similar semantics to [`AbsPath::lex_parent`], but we guarantee return value
    /// exists and is normalized i.e. any CanonicalPath that is lexically equal is guaranteed
    /// to represent the same path on disk (TOCTOU caveat).
    ///
    /// A None here would guarantee self is the root path
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prior_canonical() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let path = dir.join("does/not/exist.txt");
        let expand = ExpandPath::new(&AbsPath::new(path).unwrap()).unwrap();
        match expand {
            ExpandPath::Canonical(_) => panic!("expected partial got {:?}", expand),
            ExpandPath::Partial(PartialCanonicalPath {
                prior,
                rest,
                full: _,
            }) => {
                assert_eq!(
                    prior,
                    CanonicalPath::new(&AbsPath::new(dir).unwrap()).unwrap()
                );
                assert_eq!(rest, RelativePath::new("does/not/exist.txt").unwrap());
            }
        }
    }

    #[test]
    fn test_canonical_expand() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let expand = ExpandPath::new(&AbsPath::new(dir).unwrap()).unwrap();

        match &expand {
            ExpandPath::Canonical(canonical_path) => assert_eq!(
                canonical_path,
                &CanonicalPath::new(&AbsPath::new(dir).unwrap()).unwrap()
            ),
            ExpandPath::Partial(PartialCanonicalPath { .. }) => {
                panic!("expected full got {:?}", expand)
            }
        }
    }
}

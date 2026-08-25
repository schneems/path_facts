//! If a path can be canonicalized it exists on disk and any symlinks in it's
//! path can be resolved to entities on disk.
//!
//! It can still have other problems, such as being a file when it's
//! expected to be a directory or not having correct permissions, but
//! we can guarantee that all files involved exist.
//!
//! Built from a [`AbsPath`] so we know the program has access to CWD.
//! May have un-normalized parts i.e. `..`
use crate::{
    abs_path::AbsPath,
    component::{self, NormalComponent},
};
use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct CannotCanonicalizeAnything {
    pub(crate) original: AbsPath,
    pub(crate) root: AbsPath,
    pub(crate) root_error: std::io::Error,
}

/// What [`CanonicalPath::entry`] found at a name inside a directory
#[derive(Debug)]
pub(crate) enum Entry {
    /// Not a symlink, so the path is canonical and the metadata describes it directly
    Canonical(CanonicalPath, std::fs::Metadata),
    /// A symlink, which has to be followed before anything canonical can be said about it
    Symlink,
}

/// File exists, and is resolvable path, is fully normalized
///
/// - All symlinks resolve and are visible
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
///
/// ## Windows
///
/// On windows when you canonicalize a path you get a UNC format. i.e. `C:\` → `\\?\C:\`.
/// This is considered a path literal where every element is resolved. A property of this
/// fact is that you cannot always canonicalize a path that's been canonicalized and modified
/// i.e. `path.canonicalize().join("..\other_path").canonicalize().unwrap()` will error
/// because `..` does not exist when the path is already using UNC format.
///
/// An alternative to canonicalizing and building UNC is in the dunce crate <https://gitlab.com/kornelski/dunce/-/blob/c523a1edfa81cd7603a28971154a33c14b2fed4e/src/lib.rs>
///
/// Also take care in tests that joining `".."` literal to a `Path` will fold it in.
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

    /// Joins input with the given Canonical path without checking disk contents
    ///
    /// Unsafe because the output is a CanonicalPath, so the caller must make sure that:
    ///
    /// - The path points to a resolved location on disk
    ///
    /// The location on disk either needs to be `lstat`-able or be observed
    /// in a directory (when the directory has read, but not execute permission).
    pub(crate) unsafe fn unchecked_join(&self, rest: &NormalComponent) -> CanonicalPath {
        CanonicalPath(self.as_ref().join(rest.as_ref()))
    }

    /// Returns the filename of the path
    ///
    /// Since a canonical path is fully resolved, it will always be a normal component
    /// Returns None when the path is root (with no filename)
    pub(crate) fn filename_component(&self) -> Option<NormalComponent> {
        self.as_ref()
            .components()
            .last()
            .map(component::owned)
            .and_then(|component| component.normal().cloned())
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
    pub(crate) fn entry(&self, name: &NormalComponent) -> Result<Entry, std::io::Error> {
        let path = self.0.join(name.as_ref());
        let lstat = std::fs::symlink_metadata(&path)?;

        if lstat.file_type().is_symlink() {
            Ok(Entry::Symlink)
        } else {
            Ok(Entry::Canonical(CanonicalPath(path), lstat))
        }
    }

    /// Similar semantics to [`AbsPath::lex_parent`], but we guarantee return value
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

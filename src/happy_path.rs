//! Representations of paths with no problems
//!
//! Holds [`HappyPath`] and [`DirOk`]
use crate::{
    abs_path::{self, AbsPath},
    canonical_path::{CannotCanonicalizeAnything, CanonicalPath, ExpandPath, PartialCanonicalPath},
    resolved_metadata::{ResolvedMetadata, ResolvedType},
};
use faccess::{AccessMode, PathExt};
use std::path::Path;

/// A [`HappyPath`] represents a path with no problems that we could find
///
/// For a path to be happy, it's parent (directory) must be good too, represented by a [`DirOk`]
#[derive(Debug)]
pub(crate) struct KnownPath {
    pub(crate) absolute: AbsPath,
    pub(crate) canonical: CanonicalPath,
    pub(crate) symlink_target: Option<AbsPath>,
    pub(crate) resolved_type: ResolvedType,
    pub(crate) parent: DirOk,
    pub(crate) read: bool,
    pub(crate) write: bool,
    pub(crate) execute: bool,
}

/// A [`DirOk`] represents a directory with no problems that we could find
#[derive(Debug, Clone)]
pub(crate) struct DirOk {
    #[allow(dead_code)]
    pub(crate) absolute: AbsPath,
    pub(crate) canonical: CanonicalPath,
    pub(crate) entries: Vec<AbsPath>,
    pub(crate) read: bool,
    pub(crate) write: bool,
    pub(crate) execute: bool,
}

impl DirOk {
    pub(crate) fn new(absolute: AbsPath) -> Result<Self, std::io::Error> {
        let canonical = CanonicalPath::new(&absolute)?;
        let entries = absolute.read_dir()?;

        let read = true;
        let write = canonical.as_ref().access(AccessMode::WRITE).is_ok();
        let execute = canonical.as_ref().access(AccessMode::EXECUTE).is_ok();

        Ok(DirOk {
            absolute,
            canonical,
            entries,
            read,
            write,
            execute,
        })
    }

    pub(crate) fn has_entry(&self, path: &AbsPath) -> bool {
        self.entries.contains(path)
    }
}

#[derive(Debug)]
pub(crate) enum UnknownPath {
    AbsPathError(abs_path::AbsPathError),
    IsRoot(AbsPath),
    CannotCanonicalizeAnything(CannotCanonicalizeAnything),
    ParentProblem {
        absolute: AbsPath,
        expand: ExpandPath,
        parent: AbsPath,
        /// Original error preventing us from creating a `DirOk` for the parent directory.
        /// Not printed, we traverse prior directories to find the root cause
        _error: std::io::Error,
    },
    DoesNotExist {
        absolute: AbsPath,
        expand: ExpandPath,
        parent: DirOk,
    },
    // Path exists, but we cannot canonicalize it
    CannotCanonicalize {
        absolute: AbsPath,
        expand: ExpandPath,
        parent: DirOk,
        error: std::io::Error,
    },
    /// Path exists, but we cannot read the metadata
    /// TOCTOU likely: Path exists and can be canonicalized, but we cannot read the metadata
    ///
    /// Usually this would cause a CannotCanonicalize error, but if there is a TOCTOU race condition
    /// where the parent directory has read and execute access when the canonicalization is attempted,
    /// but loses execute access before the metadata reading, then this error will occur.
    CannotMetadata {
        absolute: AbsPath,
        canonical: CanonicalPath,
        parent: DirOk,
        error: std::io::Error,
    },
    /// Path exists, but and is reportedly a symlink but readlink fails
    /// Probably TOCTOU otherwise the canonical path would have errored
    CannotReadLink {
        absolute: AbsPath,
        canonical: CanonicalPath,
        parent: DirOk,
        error: std::io::Error,
    },
}

pub(crate) fn state(path: &Path) -> Result<KnownPath, Box<UnknownPath>> {
    let absolute = AbsPath::new(path).map_err(UnknownPath::AbsPathError)?;
    let abs_parent = absolute
        .parent()
        .ok_or_else(|| UnknownPath::IsRoot(absolute.clone()))?;
    let expand = ExpandPath::new(&absolute).map_err(UnknownPath::CannotCanonicalizeAnything)?;
    let parent = DirOk::new(abs_parent.clone()).map_err(|error| UnknownPath::ParentProblem {
        absolute: absolute.clone(),
        expand: expand.clone(),
        parent: abs_parent.clone(),
        _error: error,
    })?;
    let path_does_not_exist = !parent.has_entry(&absolute);
    let canonical = CanonicalPath::new(&absolute).map_err(|error| {
        if path_does_not_exist {
            UnknownPath::DoesNotExist {
                absolute: absolute.clone(),
                expand: expand.clone(),
                parent: parent.clone(),
            }
        } else {
            UnknownPath::CannotCanonicalize {
                absolute: absolute.clone(),
                expand: expand.clone(),
                parent: parent.clone(),
                error,
            }
        }
    })?;

    let resolved_type = ResolvedMetadata::new(&absolute)
        .map_err(|error| UnknownPath::CannotMetadata {
            absolute: absolute.clone(),
            canonical: canonical.clone(),
            parent: parent.clone(),
            error,
        })?
        .resolved_type();
    let symlink_target =
        abs_path::try_readlink(&absolute).map_err(|error| UnknownPath::CannotReadLink {
            absolute: absolute.clone(),
            canonical: canonical.clone(),
            parent: parent.clone(),
            error,
        })?;

    let read = canonical.as_ref().access(AccessMode::READ).is_ok();
    let write = canonical.as_ref().access(AccessMode::WRITE).is_ok();
    let execute = canonical.as_ref().access(AccessMode::EXECUTE).is_ok();

    Ok(KnownPath {
        absolute,
        canonical,
        symlink_target,
        resolved_type,
        parent,
        read,
        write,
        execute,
    })
}

//! Representations of paths with no problems
//!
//! Holds [`HappyPath`] and [`DirOk`]
use crate::{
    abs_path::{self, AbsPath, RelativePath},
    canonical_path::{CannotCanonicalizeAnything, CanonicalPath, ExpandPath},
    resolved_metadata::{ResolvedMetadata, ResolvedType},
    trace::{PhysicalNode, Trace},
};
use faccess::{AccessMode, PathExt};
use std::path::Path;

/// A [`HappyPath`] represents a path with no problems that we could find
///
/// For a path to be happy, it's parent (directory) must be good too, represented by a [`DirOk`]
#[derive(Debug)]
pub(crate) struct KnownPath {
    pub(crate) canonical: CanonicalPath,
    /// The resolved path as an entry in [`KnownPath::parent`]'s listing
    ///
    /// The name to annotate in the parent directory. Not [`KnownPath::canonical`]: for a
    /// symlink that is the target, not the link's own name in the parent.
    pub(crate) entry: AbsPath,
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
        #[allow(dead_code)] // Prove we can access CWD and path is not empty
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
}

pub(crate) fn state(path: &Path) -> Result<KnownPath, Box<UnknownPath>> {
    let trace = Trace::new(path).map_err(|error| match error {
        crate::trace::CannotTrace::Anchor(error) => Box::new(UnknownPath::AbsPathError(error)),
        crate::trace::CannotTrace::Root(error) => {
            Box::new(UnknownPath::CannotCanonicalizeAnything(error))
        }
    })?;
    let absolute = trace.absolute().clone();

    let abs_parent = absolute
        .lex_parent()
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
    // The walk already read this component. If it is a symlink, the `readlink` it issued is
    // recorded as the target, so there is nothing to ask the filesystem again. A trailing
    // `..` or `.` can never be a symlink, so those never land here.
    let symlink_target = match &trace.last_step().contents {
        PhysicalNode::Symlink { target, .. } => Some(target.clone()),
        _ => None,
    };

    let read = canonical.as_ref().access(AccessMode::READ).is_ok();
    let write = canonical.as_ref().access(AccessMode::WRITE).is_ok();
    let execute = canonical.as_ref().access(AccessMode::EXECUTE).is_ok();

    // The path resolved, so it sits inside the directory the walk actually reached rather
    // than its lexical parent. These differ for a trailing `..`: `<dir>/a/b/..` is spelled
    // under `<dir>/a/b` but lives in `<dir>` as the entry `a`. `entry` is that entry as it
    // appears in `parent`'s listing, which is the name to annotate and not `canonical` (a
    // symlink's `canonical` is its target, not the link's name in the parent).
    let listing = trace
        .listing()
        .ok_or_else(|| UnknownPath::IsRoot(absolute.clone()))?;
    let entry = AbsPath::from(listing.dir.clone()).join_relative(
        &RelativePath::new(&listing.entry).expect("a directory entry name is a relative path"),
    );
    let parent =
        DirOk::new(AbsPath::from(listing.dir)).map_err(|error| UnknownPath::ParentProblem {
            absolute: absolute.clone(),
            expand: expand.clone(),
            parent: abs_parent.clone(),
            _error: error,
        })?;

    Ok(KnownPath {
        canonical,
        entry,
        symlink_target,
        resolved_type,
        parent,
        read,
        write,
        execute,
    })
}

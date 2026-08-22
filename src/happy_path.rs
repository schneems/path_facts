//! Representations of paths with no problems
//!
//! Holds [`KnownPath`] and [`DirOk`]
use crate::{
    abs_path::{self, AbsPath, RelativePath},
    canonical_path::{CannotCanonicalizeAnything, CanonicalPath},
    resolved_metadata::{ResolvedMetadata, ResolvedType},
    trace::{PhysicalNode, Trace},
};
use faccess::{AccessMode, PathExt};
use std::path::Path;

/// A [`KnownPath`] represents a path with no problems that we could find
///
/// For a path to be happy, it's parent (directory) must be good too, represented by a [`DirOk`]
#[derive(Debug)]
pub(crate) struct KnownPath {
    #[allow(dead_code)]
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
        parent: AbsPath,
        /// Original error preventing us from creating a `DirOk` for the parent directory.
        /// Not printed, we traverse prior directories to find the root cause
        _error: std::io::Error,
    },
    DoesNotExist {
        absolute: AbsPath,
        parent: DirOk,
    },
    // Path exists, but we cannot canonicalize it
    CannotCanonicalize {
        absolute: AbsPath,
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
        parent: DirOk,
        error: std::io::Error,
    },
}

pub(crate) fn state_from_trace(trace: &Trace) -> Result<KnownPath, Box<UnknownPath>> {
    let absolute = trace.absolute().clone();

    let abs_parent = absolute
        .lex_parent()
        .ok_or_else(|| UnknownPath::IsRoot(absolute.clone()))?;
    let parent = DirOk::new(abs_parent.clone()).map_err(|error| UnknownPath::ParentProblem {
        absolute: absolute.clone(),
        parent: abs_parent.clone(),
        _error: error,
    })?;
    let path_does_not_exist = !parent.has_entry(&absolute);
    let symlink_target = match &trace.last_step().contents {
        // TODO represent the fact a readlink can fail to the end user somehow
        PhysicalNode::Symlink { target, .. } => target.as_ref().ok().cloned(),
        _ => None,
    };
    // Normally `canonicalize` is the arbiter of "this path fully resolves". The one case it
    // gets wrong is a trailing `..` on a Windows verbatim (`\\?\`) path: `canonicalize` does
    // not fold a `..` inside a verbatim path, so `<dir>/a/b/..` fails trying to open a literal
    // `..` entry even though it plainly resolves to `<dir>/a`. The walk folded that `..` left
    // to right (`PhysicalNode::Up`), so when the final step is an `Up` prefer its resolved
    // location. Every other shape still goes through `canonicalize`, keeping the error arms
    // (broken symlink, unsearchable directory) exactly as they were. See
    // `fact_check::tests::test_canonicalize_fails_on_trailing_dot_dot_in_a_verbatim_path`
    // for a Windows test pinning down the `canonicalize` failure this branch works around.
    let folded_dot_dot = match &trace.last_step().contents {
        PhysicalNode::Up { to, .. } => Some(to.clone()),
        _ => None,
    };
    let canonical = match folded_dot_dot {
        Some(canonical) => canonical,
        None => CanonicalPath::new(&absolute).map_err(|error| {
            if path_does_not_exist {
                UnknownPath::DoesNotExist {
                    absolute: absolute.clone(),
                    parent: parent.clone(),
                }
            } else {
                UnknownPath::CannotCanonicalize {
                    absolute: absolute.clone(),
                    parent: parent.clone(),
                    error,
                }
            }
        })?,
    };

    let resolved_type = ResolvedMetadata::new(canonical.as_ref())
        .map_err(|error| UnknownPath::CannotMetadata {
            absolute: absolute.clone(),
            parent: parent.clone(),
            error,
        })?
        .resolved_type();

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

pub(crate) fn state(path: &Path) -> Result<KnownPath, Box<UnknownPath>> {
    let trace = Trace::new(path)?;
    state_from_trace(&trace)
}

use std::path::Path;

use faccess::{AccessMode, PathExt};

use crate::{
    abs_path::{self, AbsPath},
    canonical_path::CanonicalPath,
    resolved_metadata::{ResolvedMetadata, ResolvedType},
};

#[derive(Debug)]
pub(crate) struct HappyPath {
    pub(crate) absolute: AbsPath,
    pub(crate) canonical: CanonicalPath,
    pub(crate) symlink_target: Option<AbsPath>,
    pub(crate) resolved_type: ResolvedType,
    pub(crate) parent: DirOk,
    pub(crate) read: bool,
    pub(crate) write: bool,
    pub(crate) execute: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DirOk {
    pub(crate) absolute: AbsPath,
    #[allow(dead_code)]
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

pub(crate) enum UnhappyPath {
    AbsPathError(abs_path::AbsPathError),
    IsRoot(AbsPath),
    ParentProblem {
        absolute: AbsPath,
        parent: AbsPath,
        #[allow(dead_code)]
        error: std::io::Error,
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
    /// Can happen if we have read access on the parent dir but not execute access (to view permissions)
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

pub(crate) fn state(path: &Path) -> Result<HappyPath, Box<UnhappyPath>> {
    let absolute = AbsPath::new(path).map_err(UnhappyPath::AbsPathError)?;
    let abs_parent = absolute
        .parent()
        .ok_or_else(|| UnhappyPath::IsRoot(absolute.clone()))?;
    let parent = DirOk::new(abs_parent.clone()).map_err(|error| UnhappyPath::ParentProblem {
        absolute: absolute.clone(),
        parent: abs_parent.clone(),
        error,
    })?;
    let path_does_not_exist = !parent.has_entry(&absolute);
    let canonical = CanonicalPath::new(&absolute).map_err(|error| {
        if path_does_not_exist {
            UnhappyPath::DoesNotExist {
                absolute: absolute.clone(),
                parent: parent.clone(),
            }
        } else {
            UnhappyPath::CannotCanonicalize {
                absolute: absolute.clone(),
                parent: parent.clone(),
                error,
            }
        }
    })?;

    let resolved_type = ResolvedMetadata::new(&absolute)
        .map_err(|error| UnhappyPath::CannotMetadata {
            absolute: absolute.clone(),
            canonical: canonical.clone(),
            parent: parent.clone(),
            error,
        })?
        .resolved_type();
    let symlink_target =
        abs_path::try_readlink(&absolute).map_err(|error| UnhappyPath::CannotReadLink {
            absolute: absolute.clone(),
            canonical: canonical.clone(),
            parent: parent.clone(),
            error,
        })?;

    let read = canonical.as_ref().access(AccessMode::READ).is_ok();
    let write = canonical.as_ref().access(AccessMode::WRITE).is_ok();
    let execute = canonical.as_ref().access(AccessMode::EXECUTE).is_ok();

    Ok(HappyPath {
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

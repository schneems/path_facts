//! Metadata wrapper for clarity and consistency
//!
//! A `std::fs::Metadata` struct can come from `std::fs::metadata` or `std::fs::symlink_metadata`.
//! and depending on where it comes from it's predicate (boolean) methods can have different
//! meanings.

use std::{fmt::Display, fs::Metadata, path::Path};

/// Indicates the path is a file or directory or it's a valid simlink to a file or directory
pub(crate) enum ResolvedType {
    File,
    Dir,
}

impl Display for ResolvedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedType::File => write!(f, "file"),
            ResolvedType::Dir => write!(f, "directory"),
        }
    }
}

pub(crate) struct ResolvedMetadata(Metadata);

impl ResolvedMetadata {
    pub(crate) fn new(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        std::fs::metadata(path.as_ref()).map(ResolvedMetadata)
    }

    pub(crate) fn resolved_type(&self) -> ResolvedType {
        if self.0.is_dir() {
            ResolvedType::Dir
        } else {
            ResolvedType::File
        }
    }
}

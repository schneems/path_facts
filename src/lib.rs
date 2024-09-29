use std::{
    fmt::Display,
    fs::Metadata,
    path::{Path, PathBuf},
};

use abs_path::AbsPath;
use canonical_path::CanonicalPath;
use faccess::{AccessMode, PathExt};
use resolved_metadata::{ResolvedMetadata, ResolvedType};
use style::{append_if, conditional_perms};

mod abs_path;
mod canonical_path;
mod fact_check;
mod resolved_metadata;
mod style;

#[derive(Debug, Clone)]
pub(crate) struct DirOk {
    pub(crate) absolute: AbsPath,
    pub(crate) canonical: CanonicalPath,
    pub(crate) entries: Vec<AbsPath>,
    pub(crate) read: bool,
    pub(crate) write: bool,
    pub(crate) execute: bool,
}

impl DirOk {
    fn new(absolute: AbsPath) -> Result<Self, std::io::Error> {
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

    // Must have read perms because we read the entries
    pub(crate) fn permission_caveats(&self) -> Option<String> {
        match (self.write, self.execute) {
            (true, true) => None,
            (true, false) => {
                Some("✅ read, ✅ write, ❌ execute (Cannot read metadata)".to_string())
            }
            (false, true) => {
                Some("✅ read, ❌ write, ✅ execute (Cannot modify entries)".to_string())
            }
            (false, false) => Some(
                "✅ read, ❌ write, ❌ execute (Cannot modify entries or read metadata)"
                    .to_string(),
            ),
        }
    }
}

struct HappyPath {
    absolute: AbsPath,
    canonical: CanonicalPath,
    symlink_target: Option<AbsPath>,
    resolved_type: ResolvedType,
    parent: DirOk,
    read: bool,
    write: bool,
    execute: bool,
}
enum UnhappyPath {
    AbsPathError(abs_path::AbsPathError),
    IsRoot(AbsPath),
    ParentProblem {
        absolute: AbsPath,
        parent: AbsPath,
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

pub struct PathFacts {
    path: PathBuf,
    state: Result<HappyPath, UnhappyPath>,
}

impl Display for PathFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.state {
            Ok(happy) => {
                writeln!(f, "exists `{}`", self.path.display())?;
                if let Some(target) = &happy.symlink_target {
                    writeln!(
                        f,
                        "{}",
                        style::bullet(format!("Symlink target: `{}`", target))
                    )?;
                }
                writeln!(
                    f,
                    "{}",
                    style::bullet(style::fmt_dir(&happy.parent, |entry| {
                        if entry == &happy.absolute {
                            Some(format!(
                                "({file_type}{permissions})",
                                file_type = happy.resolved_type,
                                permissions = append_if(
                                    ": ",
                                    conditional_perms(happy.read, happy.write, happy.execute)
                                )
                            ))
                        } else {
                            None
                        }
                    }))
                )?;
            }
            Err(UnhappyPath::AbsPathError(error)) => todo!(),
            Err(UnhappyPath::IsRoot(_)) => todo!(),
            Err(UnhappyPath::ParentProblem { .. }) => todo!(),
            Err(UnhappyPath::DoesNotExist { absolute, parent }) => {
                //
                writeln!(f, "does not exist `{}`", self.path.display())?;
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!(
                        "Missing `{filename}` from parent directory:\n{dir}",
                        filename = style::filename_or_path(&self.path),
                        dir = style::fmt_dir(parent, |_| { None },)
                    ))
                )?;
                if !parent.write {
                    writeln!(
                        f,
                        "{}",
                        style::bullet("Parent directory is missing write permissions (cannot create, delete, or modify files)")
                    )?;
                }
            }
            Err(UnhappyPath::CannotCanonicalize {
                absolute,
                parent,
                error,
            }) => todo!(),
            Err(UnhappyPath::CannotMetadata { .. }) => todo!(),
            Err(UnhappyPath::CannotReadLink { .. }) => todo!(),
        }

        Ok(())
    }
}

fn state(path: &Path) -> Result<HappyPath, UnhappyPath> {
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

impl PathFacts {
    pub fn new(path: impl AsRef<Path>) -> Self {
        PathFacts {
            path: path.as_ref().to_owned(),
            state: state(path.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use indoc::formatdoc;

    use super::*;
    #[allow(unused_imports)]
    use pretty_assertions::{assert_eq, assert_ne};

    #[test]
    fn test_file_exists_is_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("exists.txt");
        std::fs::write(&path, "").unwrap();

        let expected = formatdoc! {"
            exists `/path/to/directory/exists.txt`
             - `/path/to/directory`
                 └── `exists.txt` (file: ✅ read, ✅ write, ❌ execute)
        "}
        .replace(
            "/path/to/directory",
            format!("{}", tempdir.path().display()).as_str(),
        );
        let facts = PathFacts::new(path);
        assert_eq!(expected.trim(), format!("{facts}").trim());
    }

    #[test]
    fn test_parent_exists_missing_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("does_not_exist.txt");
        let expected = formatdoc! {"
            does not exist `/path/to/directory/does_not_exist.txt`
             - Missing `does_not_exist.txt` from parent directory:
               `/path/to/directory`
                  └── (empty)
        "}
        .replace(
            "/path/to/directory",
            format!("{}", tempdir.path().display()).as_str(),
        );
        let facts = PathFacts::new(path);
        assert_eq!(expected.trim(), format!("{facts}").trim());
    }
}

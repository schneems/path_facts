use crate::abs_path::AbsPathError;
use crate::happy_path::{state, HappyPath, UnhappyPath};
use crate::resolved_metadata::ResolvedType;
use crate::style::{self, conditional_perms};
use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

/// Shows helpful facts about a path when `Display`ed.
pub struct PathFacts {
    path: PathBuf,
    state: Result<HappyPath, Box<UnhappyPath>>,
}

impl PathFacts {
    pub fn new(path: impl AsRef<Path>) -> Self {
        PathFacts {
            path: path.as_ref().to_owned(),
            state: state(path.as_ref()),
        }
    }
}

impl Display for PathFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.state.as_ref().map_err(|e| &**e) {
            Ok(happy) => {
                writeln!(f, "exists `{}`", self.path.display())?;
                if self.path.is_relative() {
                    writeln!(
                        f,
                        "{}",
                        style::bullet(format!("Absolute: {absolute}", absolute = happy.absolute))
                    )?;
                }

                if let Some(target) = &happy.symlink_target {
                    writeln!(
                        f,
                        "{}",
                        style::bullet(format!("Canonical: {}", happy.canonical))
                    )?;
                    writeln!(
                        f,
                        "{}",
                        style::bullet(format!("Symlink target: {}", target))
                    )?;
                }
                writeln!(
                    f,
                    "{}",
                    style::bullet(style::fmt_dir(&happy.parent, |entry| {
                        if entry == &happy.absolute {
                            Some(format!(
                                "{file_type} [{permissions}]",
                                file_type = happy.resolved_type,
                                permissions =
                                    conditional_perms(happy.read, happy.write, happy.execute)
                            ))
                        } else {
                            None
                        }
                    }))
                )?;
            }
            Err(UnhappyPath::AbsPathError(AbsPathError::PathIsEmpty(path))) => {
                writeln!(f, "path `{}` is empty", path.display())?;
            }
            Err(UnhappyPath::AbsPathError(AbsPathError::CannotReadCWD(path, error))) => {
                writeln!(f, "`{}`", path.display())?;
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot read current working directory: {}", error))
                )?;
            }
            Err(UnhappyPath::IsRoot(absolute)) => {
                writeln!(f, "is root {absolute}")?;
            }
            Err(UnhappyPath::ParentProblem {
                absolute,
                parent,
                error: _,
            }) => {
                writeln!(f, "cannot access `{}`", self.path.display())?;
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {absolute}",)))?;
                }

                let mut prior_dir = parent.clone();
                let mut prior_state = state(parent.as_ref());
                while let Err(UnhappyPath::ParentProblem {
                    absolute: _,
                    parent,
                    error: _,
                }) = prior_state.as_ref().map_err(|e| &**e)
                {
                    prior_dir = parent.clone();
                    prior_state = state(prior_dir.as_ref());
                }
                match &prior_state {
                    Ok(HappyPath {
                        resolved_type: ResolvedType::File,
                        ..
                    }) => {
                        writeln!(f, "{}", style::bullet("Prior path is not a directory"))?;
                        writeln!(
                            f,
                            "{}",
                            style::bullet(format!(
                                "Prior path {}",
                                PathFacts::new(prior_dir.as_ref())
                            ))
                        )?
                    }
                    _ => {
                        writeln!(
                            f,
                            "{}",
                            style::bullet(format!(
                                "Prior directory {}",
                                PathFacts::new(prior_dir.as_ref())
                            ))
                        )?;
                    }
                }
            }
            Err(UnhappyPath::DoesNotExist { absolute, parent }) => {
                writeln!(f, "does not exist `{}`", self.path.display())?;
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {absolute}",)))?;
                }

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
            }) => {
                if parent.has_entry(absolute) {
                    writeln!(f, "exists `{}`", self.path.display())?;
                } else {
                    writeln!(f, "does not exist `{}`", self.path.display())?;
                }
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {absolute}",)))?;
                }
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot canonicalize due to error `{error}`",))
                )?;
                writeln!(
                    f,
                    "{}",
                    style::bullet(style::fmt_dir(parent, |entry| {
                        if entry == absolute {
                            Some("(exists)".to_string())
                        } else {
                            None
                        }
                    }))
                )?;
            }
            Err(UnhappyPath::CannotMetadata {
                absolute,
                canonical,
                parent,
                error,
            }) => {
                if parent.has_entry(absolute) {
                    writeln!(f, "exists `{}`", self.path.display())?;
                } else {
                    writeln!(f, "does not exist `{}`", self.path.display())?;
                }
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {absolute}",)))?;
                }
                writeln!(f, "{}", style::bullet(format!("Canonical: {canonical}",)))?;
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot read metadata due to error `{error}`",))
                )?;
                writeln!(
                    f,
                    "{}",
                    style::bullet(style::fmt_dir(parent, |entry| {
                        if entry == absolute {
                            Some("(exists)".to_string())
                        } else {
                            None
                        }
                    }))
                )?;
            }
            Err(UnhappyPath::CannotReadLink {
                absolute,
                canonical,
                parent,
                error,
            }) => {
                if parent.has_entry(absolute) {
                    writeln!(f, "exists `{}`", self.path.display())?;
                } else {
                    writeln!(f, "does not exist `{}`", self.path.display())?;
                }
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {absolute}",)))?;
                }
                writeln!(f, "{}", style::bullet(format!("Canonical: {canonical}",)))?;
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot readlink due to error `{error}`",))
                )?;
                writeln!(
                    f,
                    "{}",
                    style::bullet(style::fmt_dir(parent, |entry| {
                        if entry == absolute {
                            Some("(exists)".to_string())
                        } else {
                            None
                        }
                    }))
                )?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_help::SetCurrentDirTempSafe;

    #[test]
    fn test_prior_dir_problem_is_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        std::fs::write(tempdir.path().join("a"), "").unwrap();

        insta::with_settings!({prepend_module_to_snapshot => false}, {
            insta::assert_snapshot!(
                "prior_dir_problem_is_file",
                PathFacts::new(path)
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory")
            );
        });

        // Verify README doesn't need to be updated
        assert!(
            include_str!("../README.md").contains(
                include_str!("snapshots/prior_dir_problem_is_file.snap")
                    .split("---")
                    .nth(2)
                    .expect("Snapshot should have YAML frontmatter")
                    .trim()
            ),
            "README missing correct example output. Update the module docs and re-run `cargo rdme`"
        );
    }

    #[test]
    fn test_prior_dir_problem_does_not_exist() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory"),
            @r"
            cannot access `/path/to/directory/a/b/c/does_not_exist.txt`
             - Prior directory does not exist `/path/to/directory/a`
                - Missing `a` from parent directory:
                  `/path/to/directory`
                     └── (empty)
            ")
    }

    #[test]
    fn test_empty_path() {
        insta::assert_snapshot!(
            PathFacts::new(Path::new("")),
            @"path `` is empty"
        )
    }

    #[test]
    fn test_file_exists_is_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("exists.txt");
        std::fs::write(&path, "").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory"),
            @r"
            exists `/path/to/directory/exists.txt`
             - `/path/to/directory`
                 └── `exists.txt` file [✅ read, ✅ write, ❌ execute]
            ")
    }

    #[test]
    fn test_parent_exists_missing_file() {
        let tempdir = tempfile::tempdir().unwrap();
        insta::assert_snapshot!(
            PathFacts::new(tempdir.path().join("does_not_exist.txt"))
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory"),
            @r"
            does not exist `/path/to/directory/does_not_exist.txt`
             - Missing `does_not_exist.txt` from parent directory:
               `/path/to/directory`
                  └── (empty)
            ")
    }

    #[test]
    fn test_rename_two_missing_paths() {
        use indoc::formatdoc;

        let temp = SetCurrentDirTempSafe::new();

        let from = std::path::Path::new("doesnotexist.txt");
        let to = std::path::Path::new("also_does_not_exist.txt");

        let result = std::fs::rename(from, to).map_err(|_error| {
            formatdoc! {"
            cannot rename from `{}` to `{}` due to: {{error}}.

            From path {from_facts}
            To path {to_facts}
            ",
                from.display(),
                to.display(),
                from_facts = PathFacts::new(from),
                to_facts = PathFacts::new(to)
            }
        });

        insta::with_settings!({prepend_module_to_snapshot => false}, {
            insta::assert_snapshot!(
                "rename_two_missing_paths",
                result.unwrap_err()
                    .to_string()
                    .replace(&temp.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
            );
        });
    }

    #[test]
    fn test_relative_path_exists() {
        let temp = SetCurrentDirTempSafe::new();

        let path = Path::new("exists.txt");
        std::fs::write(path, "").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&temp.path().canonicalize().unwrap().display().to_string(), "/path/to/directory"),
            @r"
            exists `exists.txt`
             - Absolute: `/path/to/directory/exists.txt`
             - `/path/to/directory`
                 └── `exists.txt` file [✅ read, ✅ write, ❌ execute]
            ")
    }

    #[test]
    fn test_symlink_to_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let target_file = tempdir.path().join("target.txt");
        let symlink_path = tempdir.path().join("link_to_target.txt");

        std::fs::write(&target_file, "content").unwrap();
        std::os::unix::fs::symlink(&target_file, &symlink_path).unwrap();

        let canonical = tempdir.path().canonicalize().unwrap();
        let output = PathFacts::new(&symlink_path)
            .to_string()
            .replace(&canonical.display().to_string(), "/path/to/canonical")
            .replace(&tempdir.path().display().to_string(), "/path/to/directory");

        insta::assert_snapshot!(
            output,
            @r"
             exists `/path/to/directory/link_to_target.txt`
              - Canonical: `/path/to/canonical/target.txt`
              - Symlink target: `/path/to/directory/target.txt`
              - `/path/to/directory`
                  ├── `target.txt`
                  └── `link_to_target.txt` file [✅ read, ✅ write, ❌ execute]
        ");
    }

    #[test]
    fn test_symlink_to_directory() {
        let tempdir = tempfile::tempdir().unwrap();
        let target_dir = tempdir.path().join("target_dir");
        let symlink_path = tempdir.path().join("link_to_dir");

        std::fs::create_dir(&target_dir).unwrap();
        std::os::unix::fs::symlink(&target_dir, &symlink_path).unwrap();

        let canonical = tempdir.path().canonicalize().unwrap();
        let output = PathFacts::new(&symlink_path)
            .to_string()
            .replace(&canonical.display().to_string(), "/path/to/canonical")
            .replace(&tempdir.path().display().to_string(), "/path/to/directory");

        insta::assert_snapshot!(
            output,
            @r"
             exists `/path/to/directory/link_to_dir`
              - Canonical: `/path/to/canonical/target_dir`
              - Symlink target: `/path/to/directory/target_dir`
              - `/path/to/directory`
                  ├── `link_to_dir` directory []
                  └── `target_dir`
        ");
    }
}

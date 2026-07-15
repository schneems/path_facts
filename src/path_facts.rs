//! Facts about paths
use crate::abs_path::AbsPathError;
use crate::happy_path::{state, HappyPath, UnhappyPath};
use crate::resolved_metadata::ResolvedType;
use crate::style::{self, permissions};
use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

/// Shows helpful facts about a path when `Display`ed.
pub struct PathFacts {
    /// Original input path
    path: PathBuf,
    /// Detected state of the path
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
        self.fmt_individual_facts(f)?;
        self.fmt_parent_facts(f)?;
        Ok(())
    }
}

impl PathFacts {
    fn fmt_individual_facts(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
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
            }
            Err(UnhappyPath::AbsPathError(AbsPathError::PathIsEmpty(path))) => {
                writeln!(f, "path `{}` is empty", path.display())?;
            }
            Err(UnhappyPath::AbsPathError(AbsPathError::CannotReadCWD(path, _))) => {
                writeln!(f, "`{}`", path.display())?;
            }
            Err(UnhappyPath::IsRoot(absolute)) => {
                writeln!(f, "is root {absolute}")?;
            }
            Err(UnhappyPath::ParentProblem {
                absolute,
                parent: _,
                _error,
            }) => {
                writeln!(f, "cannot access `{}`", self.path.display())?;
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {absolute}",)))?;
                }
            }
            Err(UnhappyPath::DoesNotExist {
                absolute,
                parent: _,
            }) => {
                writeln!(f, "does not exist `{}`", self.path.display())?;
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {absolute}",)))?;
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
            }
        }

        Ok(())
    }

    fn fmt_parent_facts(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        match self.state.as_ref().map_err(|e| &**e) {
            Ok(happy) => {
                writeln!(
                    f,
                    "{}",
                    style::bullet(style::fmt_dir(&happy.parent, |entry| {
                        if entry == &happy.absolute {
                            Some(format!(
                                "{file_type} {permissions}",
                                file_type = happy.resolved_type,
                                permissions = permissions(happy.read, happy.write, happy.execute)
                            ))
                        } else {
                            None
                        }
                    }))
                )?;
            }
            Err(UnhappyPath::AbsPathError(AbsPathError::PathIsEmpty(_))) => {}
            Err(UnhappyPath::AbsPathError(AbsPathError::CannotReadCWD(_, error))) => {
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot read current working directory: {}", error))
                )?;
            }
            Err(UnhappyPath::IsRoot(_)) => {}
            Err(UnhappyPath::ParentProblem {
                absolute: _,
                parent,
                _error,
            }) => {
                let mut prior_dir = parent.clone();
                let mut prior_state = state(parent.as_ref());
                while let Err(UnhappyPath::ParentProblem {
                    absolute: _,
                    parent,
                    _error,
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
                        writeln!(
                            f,
                            "{}",
                            style::bullet(format!("Prior path is not a directory {prior_dir}"))
                        )?;

                        let mut parent_facts = String::new();
                        PathFacts {
                            path: prior_dir.as_ref().to_owned(),
                            state: prior_state,
                        }
                        .fmt_parent_facts(&mut parent_facts)?;
                        writeln!(
                            f,
                            "{}",
                            style::prefix_first_rest_lines("   ", "   ", &parent_facts)
                        )?
                    }
                    _ => {
                        writeln!(
                            f,
                            "{}",
                            style::bullet(format!(
                                "Prior directory {}",
                                PathFacts {
                                    path: prior_dir.as_ref().to_owned(),
                                    state: prior_state
                                }
                            ))
                        )?;
                    }
                }
            }
            Err(UnhappyPath::DoesNotExist { absolute, parent })
            | Err(UnhappyPath::CannotCanonicalize {
                absolute,
                parent,
                error: _,
            })
            | Err(UnhappyPath::CannotMetadata {
                absolute,
                canonical: _,
                parent,
                error: _,
            })
            | Err(UnhappyPath::CannotReadLink {
                absolute,
                canonical: _,
                parent,
                error: _,
            }) => {
                if !parent.write {
                    writeln!(
                        f,
                        "{}",
                        style::bullet("Parent directory is missing write permissions (cannot create, delete, or modify files)")
                    )?;
                }

                if parent.has_entry(absolute) {
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
                } else {
                    writeln!(
                        f,
                        "{}",
                        style::bullet(format!(
                            "Missing `{filename}` from parent directory:\n{dir}",
                            filename = style::filename_or_path(&self.path),
                            dir = style::fmt_dir(parent, |_| { None },)
                        ))
                    )?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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
    }

    #[test]
    fn verify_rdme_updated() {
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

        let tempdir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();

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
                    .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
            );
        });
    }

    #[test]
    fn test_relative_path_exists() {
        let tempdir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();

        let path = Path::new("exists.txt");
        std::fs::write(path, "").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory"),
            @r"
            exists `exists.txt`
             - Absolute: `/path/to/directory/exists.txt`
             - `/path/to/directory`
                 └── `exists.txt` file [✅ read, ✅ write, ❌ execute]
            ")
    }

    #[test]
    fn test_symlink_to_file() {
        // Use two separate temp directories to guarantee different paths on all platforms
        let target_tempdir = tempfile::tempdir().unwrap();
        let link_tempdir = tempfile::tempdir().unwrap();

        // Create target file in first tempdir
        let target_file = target_tempdir.path().join("target.txt");
        std::fs::write(&target_file, "content").unwrap();

        // Create symlink in second tempdir pointing to first tempdir
        let symlink_path = link_tempdir.path().join("link_to_target.txt");
        std::os::unix::fs::symlink(&target_file, &symlink_path).unwrap();

        let target_canonical = target_tempdir.path().canonicalize().unwrap();
        let link_canonical = link_tempdir.path().canonicalize().unwrap();

        let output = PathFacts::new(&symlink_path)
            .to_string()
            .replace(&target_canonical.display().to_string(), "/path/to/target")
            .replace(
                &target_tempdir.path().display().to_string(),
                "/path/to/target",
            )
            .replace(&link_canonical.display().to_string(), "/path/to/link")
            .replace(&link_tempdir.path().display().to_string(), "/path/to/link");

        insta::assert_snapshot!(
            output,
            @r"
             exists `/path/to/link/link_to_target.txt`
              - Canonical: `/path/to/target/target.txt`
              - Symlink target: `/path/to/target/target.txt`
              - `/path/to/link`
                  └── `link_to_target.txt` file [✅ read, ✅ write, ❌ execute]
        ");
    }

    #[test]
    fn test_symlink_to_directory() {
        // Use two separate temp directories to guarantee different paths on all platforms
        let target_tempdir = tempfile::tempdir().unwrap();
        let link_tempdir = tempfile::tempdir().unwrap();

        // Create target directory in first tempdir
        let target_dir = target_tempdir.path().join("target_dir");
        std::fs::create_dir(&target_dir).unwrap();

        // Create symlink in second tempdir pointing to first tempdir
        let symlink_path = link_tempdir.path().join("link_to_dir");
        std::os::unix::fs::symlink(&target_dir, &symlink_path).unwrap();

        let target_canonical = target_tempdir.path().canonicalize().unwrap();
        let link_canonical = link_tempdir.path().canonicalize().unwrap();

        let output = PathFacts::new(&symlink_path)
            .to_string()
            .replace(&target_canonical.display().to_string(), "/path/to/target")
            .replace(
                &target_tempdir.path().display().to_string(),
                "/path/to/target",
            )
            .replace(&link_canonical.display().to_string(), "/path/to/link")
            .replace(&link_tempdir.path().display().to_string(), "/path/to/link");

        insta::assert_snapshot!(
            output,
            @r"
             exists `/path/to/link/link_to_dir`
              - Canonical: `/path/to/target/target_dir`
              - Symlink target: `/path/to/target/target_dir`
              - `/path/to/link`
                  └── `link_to_dir` directory [✅ read, ✅ write, ✅ execute]
        ");
    }

    #[test]
    fn test_cannot_read_cwd() {
        let tempdir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();

        // Remove the current working directory while we're still in it
        std::fs::remove_dir(tempdir.path()).unwrap();

        insta::assert_snapshot!(
            PathFacts::new("relative_path.txt")
                .to_string()
                .replace(
                    &std::fs::read_to_string(tempdir.path()).unwrap_err().to_string(),
                    "{error}"
                ),
            @r"
            `relative_path.txt`
             - Cannot read current working directory: {error}
            ");
    }

    #[test]
    fn test_is_root() {
        insta::assert_snapshot!(
            PathFacts::new("/"),
            @"is root `/`"
        );
    }

    #[test]
    fn test_prior_dir_problem_relative_path() {
        let tempdir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();

        insta::assert_snapshot!(
            // Create a relative path where the parent directories don't exist
            PathFacts::new(Path::new("a/b/c/does_not_exist.txt"))
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory"),
            @r"
            cannot access `a/b/c/does_not_exist.txt`
             - Absolute: `/path/to/directory/a/b/c/does_not_exist.txt`
             - Prior directory does not exist `/path/to/directory/a`
                - Missing `a` from parent directory:
                  `/path/to/directory`
                     └── (empty)
            ");
    }

    #[test]
    #[cfg(unix)]
    fn test_parent_directory_missing_write_permissions() {
        let tempdir = tempfile::tempdir().unwrap();
        let readonly_dir = tempdir.path().join("readonly_dir");
        std::fs::create_dir(&readonly_dir).unwrap();

        // Remove write permissions from the directory
        let mut perms = std::fs::metadata(&readonly_dir).unwrap().permissions();
        perms.set_mode(0o555); // read + execute, no write
        std::fs::set_permissions(&readonly_dir, perms).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(readonly_dir.join("does_not_exist.txt"))
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory"),
            @r"
            does not exist `/path/to/directory/readonly_dir/does_not_exist.txt`
             - Parent directory is missing write permissions (cannot create, delete, or modify files)
             - Missing `does_not_exist.txt` from parent directory:
               `/path/to/directory/readonly_dir` [✅ read, ❌ write, ✅ execute]
                  └── (empty)
            "
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_circular_symlink_absolute() {
        let tempdir = tempfile::tempdir().unwrap();
        let link1 = tempdir.path().join("link1");
        let link2 = tempdir.path().join("link2");

        // Create circular symlinks
        std::os::unix::fs::symlink(&link2, &link1).unwrap();
        std::os::unix::fs::symlink(&link1, &link2).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(&link1)
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize(&link1).unwrap_err().to_string(), "{error}"),
            @r"
            exists `/path/to/directory/link1`
             - Cannot canonicalize due to error `{error}`
             - `/path/to/directory`
                 ├── `link1` (exists)
                 └── `link2`
            "
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_circular_symlink_relative() {
        let tempdir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();

        // Create circular symlinks with relative paths
        std::os::unix::fs::symlink("link2", "link1").unwrap();
        std::os::unix::fs::symlink("link1", "link2").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(Path::new("link1"))
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize("link1").unwrap_err().to_string(), "{error}"),
            @r"
            exists `link1`
             - Absolute: `/path/to/directory/link1`
             - Cannot canonicalize due to error `{error}`
             - `/path/to/directory`
                 ├── `link1` (exists)
                 └── `link2`
            "
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_broken_symlink_absolute() {
        let tempdir = tempfile::tempdir().unwrap();
        let broken_link = tempdir.path().join("broken_link");
        let nonexistent = tempdir.path().join("does_not_exist");

        // Create a symlink pointing to a non-existent target
        std::os::unix::fs::symlink(&nonexistent, &broken_link).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(&broken_link)
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize(&broken_link).unwrap_err().to_string(), "{error}"),
            @r"
            exists `/path/to/directory/broken_link`
             - Cannot canonicalize due to error `{error}`
             - `/path/to/directory`
                 └── `broken_link` (exists)
            "
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_broken_symlink_relative() {
        let tempdir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();

        // Create a symlink pointing to a non-existent target (relative path)
        std::os::unix::fs::symlink("does_not_exist", "broken_link").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(Path::new("broken_link"))
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize("broken_link").unwrap_err().to_string(), "{error}"),
            @r"
            exists `broken_link`
             - Absolute: `/path/to/directory/broken_link`
             - Cannot canonicalize due to error `{error}`
             - `/path/to/directory`
                 └── `broken_link` (exists)
            "
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_no_execute_dir_with_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let no_exec_dir = tempdir.path().join("no_exec_dir");
        std::fs::create_dir(&no_exec_dir).unwrap();

        let file = no_exec_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        // Remove execute permission from directory (can read dir but not traverse)
        let mut perms = std::fs::metadata(&no_exec_dir).unwrap().permissions();
        perms.set_mode(0o644); // read + write, no execute
        std::fs::set_permissions(&no_exec_dir, perms).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(&file)
                .to_string()
                .replace(&tempdir.path().display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize(&file).unwrap_err().to_string(), "{error}"),
            @r"
            exists `/path/to/directory/no_exec_dir/file.txt`
             - Cannot canonicalize due to error `{error}`
             - `/path/to/directory/no_exec_dir` [✅ read, ✅ write, ❌ execute]
                 └── `file.txt` (exists)
            "
        );
    }
}

use crate::abs_path::AbsPathError;
use crate::happy_path::{state, HappyPath, UnhappyPath};
use crate::resolved_metadata::ResolvedType;
use crate::style::{self, append_if, conditional_perms};
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
    use indoc::formatdoc;

    use super::*;
    #[allow(unused_imports)]
    use pretty_assertions::{assert_eq, assert_ne};

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
        let expected = formatdoc! {"
            cannot access `/path/to/directory/a/b/c/does_not_exist.txt`
             - Prior path is not a directory
             - Prior path exists `/path/to/directory/a`
                - `/path/to/directory`
                    └── `a` (file: ✅ read, ✅ write, ❌ execute)
        "}
        .replace(
            "/path/to/directory",
            format!("{}", tempdir.path().display()).as_str(),
        );
        let facts = PathFacts::new(path);

        println!("{:?}", expected.trim());
        println!("{:?}", format!("{facts}").trim());
        assert_eq!(expected.trim(), format!("{facts}").trim());
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
        let expected = formatdoc! {"
            cannot access `/path/to/directory/a/b/c/does_not_exist.txt`
             - Prior directory does not exist `/path/to/directory/a`
                - Missing `a` from parent directory:
                  `/path/to/directory`
                     └── (empty)
        "}
        .replace(
            "/path/to/directory",
            format!("{}", tempdir.path().display()).as_str(),
        );

        let facts = PathFacts::new(path);
        println!("{:?}", expected.trim());
        println!("{:?}", format!("{facts}").trim());
        assert_eq!(expected.trim(), format!("{facts}").trim());
    }

    #[test]
    fn test_empty_path() {
        let path = Path::new("");

        let expected = formatdoc! {"
            path `` is empty
        "};
        let facts = PathFacts::new(path);
        assert_eq!(expected.trim(), format!("{facts}").trim());
    }

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

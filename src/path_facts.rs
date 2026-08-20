//! Facts about paths
use crate::abs_path::AbsPathError;
use crate::canonical_path::{CannotCanonicalizeAnything, ExpandPath};
use crate::happy_path::{state, KnownPath, UnknownPath};
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
    state: Result<KnownPath, Box<UnknownPath>>,
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
        let mut buf = String::new();
        self.write_facts(&mut buf)?;
        writeln!(f, "{}", buf.trim_end_matches('\n'))
    }
}

impl PathFacts {
    fn write_facts(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        self.fmt_individual_facts(f)?;
        self.fmt_parent_facts(f)?;
        Ok(())
    }

    fn fmt_individual_facts(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        match self.state.as_ref().map_err(|e| &**e) {
            Ok(happy) => {
                let expanded = ExpandPath::from(happy.canonical.clone());
                writeln!(f, "exists {}", style::expanded(&self.path, &expanded))?;

                if let Some(target) = &happy.symlink_target {
                    writeln!(
                        f,
                        "{}",
                        style::bullet(format!("Symlink target: {}", target))
                    )?;
                }
            }
            Err(UnknownPath::AbsPathError(AbsPathError::PathIsEmpty(path))) => {
                writeln!(f, "path `{}` is empty", path.display())?;
            }
            Err(UnknownPath::AbsPathError(AbsPathError::CannotReadCWD(path, _))) => {
                writeln!(f, "`{}`", path.display())?;
            }
            Err(UnknownPath::CannotCanonicalizeAnything(CannotCanonicalizeAnything {
                original,
                ..
            })) => {
                writeln!(f, "`{}`", &self.path.display())?;
                if self.path.is_relative() {
                    writeln!(f, "{}", style::bullet(format!("Absolute: {original}",)))?;
                };
                // Error is in root, show message in the parent facts
            }
            Err(UnknownPath::IsRoot(absolute)) => {
                writeln!(f, "is root {absolute}")?;
            }
            Err(UnknownPath::ParentProblem {
                absolute: _,
                expand,
                parent: _,
                _error,
            }) => {
                writeln!(f, "cannot access {}", style::expanded(&self.path, expand))?;
            }
            Err(UnknownPath::DoesNotExist {
                absolute: _,
                expand,
                parent: _,
            }) => {
                writeln!(f, "does not exist {}", style::expanded(&self.path, expand))?;
            }
            Err(UnknownPath::CannotCanonicalize {
                absolute,
                expand,
                parent,
                error,
            }) => {
                if parent.has_entry(absolute) {
                    writeln!(f, "exists {}", style::expanded(&self.path, expand))?;
                } else {
                    writeln!(f, "does not exist {}", style::expanded(&self.path, expand))?;
                }
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot canonicalize due to error `{error}`",))
                )?;
            }
            Err(UnknownPath::CannotMetadata {
                absolute,
                canonical,
                parent,
                error,
            }) => {
                let expanded = ExpandPath::from(canonical.clone());
                if parent.has_entry(absolute) {
                    writeln!(f, "exists {}", style::expanded(&self.path, &expanded))?;
                } else {
                    writeln!(
                        f,
                        "does not exist {}",
                        style::expanded(&self.path, &expanded)
                    )?;
                }
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot read metadata due to error `{error}`",))
                )?;
            }
            Err(UnknownPath::CannotReadLink {
                absolute,
                canonical,
                parent,
                error,
            }) => {
                let expanded = ExpandPath::from(canonical.clone());
                if parent.has_entry(absolute) {
                    writeln!(f, "exists {}", style::expanded(&self.path, &expanded))?;
                } else {
                    writeln!(
                        f,
                        "does not exist {}",
                        style::expanded(&self.path, &expanded)
                    )?;
                }
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
            Err(UnknownPath::AbsPathError(AbsPathError::PathIsEmpty(_))) => {}
            Err(UnknownPath::AbsPathError(AbsPathError::CannotReadCWD(_, error))) => {
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!("Cannot read current working directory: {}", error))
                )?;
            }
            Err(UnknownPath::IsRoot(_)) => {}
            Err(UnknownPath::CannotCanonicalizeAnything(CannotCanonicalizeAnything {
                original: _,
                root,
                root_error,
            })) => {
                writeln!(
                    f,
                    "{}",
                    style::bullet(format!(
                        "Cannot canonicalize root {root} due to error: {root_error}"
                    ))
                )?;
            }
            Err(UnknownPath::ParentProblem {
                absolute: _,
                expand: _,
                parent,
                _error,
            }) => {
                let mut prior_dir = parent.clone();
                let mut prior_state = state(parent.as_ref());
                while let Err(UnknownPath::ParentProblem {
                    absolute: _,
                    expand: _,
                    parent,
                    _error,
                }) = prior_state.as_ref().map_err(|e| &**e)
                {
                    prior_dir = parent.clone();
                    prior_state = state(prior_dir.as_ref());
                }
                match &prior_state {
                    Ok(KnownPath {
                        resolved_type: ResolvedType::File,
                        ..
                    }) => {
                        writeln!(
                            f,
                            "{}",
                            style::bullet(format!("Prior path is not a directory {prior_dir}"))
                        )?;

                        // We've already stated the prior path (and that it's a file) above, so
                        // emit only its parent directory listing here. Using `write_facts` would
                        // repeat the redundant `exists ...` individual-fact line.
                        let mut parent_facts = String::new();
                        PathFacts {
                            path: prior_dir.as_ref().to_owned(),
                            state: prior_state,
                        }
                        .fmt_parent_facts(&mut parent_facts)?;
                        // Use `write!` because `parent_facts` already has a newline at the end.
                        write!(
                            f,
                            "{}",
                            style::prefix_first_rest_lines("   ", "   ", &parent_facts)
                        )?
                    }
                    _ => {
                        // The prior path hasn't been described yet, so emit its full facts
                        // (individual + parent), e.g. `does not exist ...` plus the dir listing.
                        let mut prior = String::new();
                        PathFacts {
                            path: prior_dir.as_ref().to_owned(),
                            state: prior_state,
                        }
                        .write_facts(&mut prior)?;
                        writeln!(f, "{}", style::bullet(format!("Prior directory {prior}")))?;
                    }
                }
            }
            Err(UnknownPath::DoesNotExist {
                absolute,
                expand: _,
                parent,
            })
            | Err(UnknownPath::CannotCanonicalize {
                absolute,
                expand: _,
                parent,
                error: _,
            })
            | Err(UnknownPath::CannotMetadata {
                absolute,
                canonical: _,
                parent,
                error: _,
            })
            | Err(UnknownPath::CannotReadLink {
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
    use crate::abs_path::AbsPath;
    use crate::canonical_path::{CanonicalPath, ExpandPath};
    use crate::happy_path::DirOk;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    // Cross-platform symlink creation for tests. Unix has a single `symlink` that
    // ignores the target's type; Windows splits it into `symlink_file` and
    // `symlink_dir` and needs the right one chosen up front. These wrappers pick
    // the correct call per OS so the tests below can run on both.
    //
    // Creating a symlink on Windows requires SeCreateSymbolicLinkPrivilege (admin
    // or Developer Mode, which GitHub's runners enable).
    fn symlink_file<P: AsRef<Path>, Q: AsRef<Path>>(target: P, link: Q) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, link)
        }
    }

    fn symlink_dir<P: AsRef<Path>, Q: AsRef<Path>>(target: P, link: Q) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link)
        }
    }

    // Cross-platform "remove write permission" for tests. Unix drops the write mode
    // bit (keeping read+execute so the directory is still traversable); Windows sets
    // the read-only attribute. Both make `access(WRITE)` report the directory as
    // not writable, which is what the tests observe.
    fn set_read_only<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            set_mode(path, 0o555) // read + execute, no write
        }
        #[cfg(windows)]
        {
            let path = path.as_ref();
            let mut perms = std::fs::metadata(path)?.permissions();
            perms.set_readonly(true);
            std::fs::set_permissions(path, perms)
        }
    }

    // Unix-only mode helpers. These set specific POSIX mode bits that have no
    // Windows equivalent, so the tests that use them are `#[cfg(unix)]`.
    #[cfg(unix)]
    fn set_mode<P: AsRef<Path>>(path: P, mode: u32) -> std::io::Result<()> {
        let path = path.as_ref();
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(path, perms)
    }

    #[test]
    fn test_prior_dir_problem_is_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let path = dir.join("a").join("b").join("c").join("does_not_exist.txt");

        std::fs::write(dir.join("a"), "").unwrap();

        insta::with_settings!({prepend_module_to_snapshot => false}, {
            insta::assert_snapshot!(
                "prior_dir_problem_is_file",
                PathFacts::new(path)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑"
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
                    .replace("🛑", "")
                    .trim()
            ),
            "README missing correct example output. Update the module docs and re-run `cargo rdme`"
        );
    }

    #[test]
    fn test_prior_dir_problem_does_not_exist() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let path = dir.join("a").join("b").join("c").join("does_not_exist.txt");

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        cannot access `/path/to/directory/a/b/c/does_not_exist.txt`
         - Prior directory does not exist `/path/to/directory/a`
            - Missing `a` from parent directory:
              `/path/to/directory`
                 └── (empty)
        🛑
        ")
    }

    #[test]
    fn test_empty_path() {
        insta::assert_snapshot!(
            PathFacts::new(Path::new("")).to_string() + "🛑",
            @r"
        path `` is empty
        🛑
        "
        )
    }

    #[test]
    fn test_file_exists_is_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let path = dir.join("exists.txt");
        std::fs::write(&path, "").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        exists `/path/to/directory/exists.txt`
         - `/path/to/directory`
             └── `exists.txt` file [✅ read, ✅ write, ❌ execute]
        🛑
        ")
    }

    #[test]
    fn test_exists_dot_dot_annotates_resolved_entry() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let b = dir.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("inside.txt"), "").unwrap();
        std::fs::write(dir.join("other.txt"), "").unwrap();

        // `<dir>/a/b/..` resolves to the directory `<dir>/a`, so the parent facts should list
        // `<dir>` and annotate the `a` entry. Parent facts come from the lexical parent
        // (`<dir>/a/b`) rather than the resolved path, so the wrong directory is listed and
        // `read_dir` never yields a `..` entry to match the un-normalized absolute path,
        // dropping the file type and permissions annotation.
        let path = b.join("..");

        insta::assert_snapshot!(
            PathFacts::new(&path)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory") + "🛑",
            @r"
        exists `/path/to/directory/a/b/..` → `/path/to/directory/a`
         - `/path/to/directory`
             ├── `a` directory [✅ read, ✅ write, ✅ execute]
             └── `other.txt`
        🛑
        ")
    }

    #[test]
    fn test_parent_exists_missing_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        insta::assert_snapshot!(
            PathFacts::new(dir.join("does_not_exist.txt"))
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        does not exist `/path/to/directory/does_not_exist.txt`
         - Missing `does_not_exist.txt` from parent directory:
           `/path/to/directory`
              └── (empty)
        🛑
        ")
    }

    #[test]
    fn test_rename_two_missing_paths() {
        use indoc::formatdoc;

        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        std::env::set_current_dir(dir).unwrap();

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
                    .replace('\\', "/") + "🛑"
            );
        });
    }

    #[test]
    fn test_relative_path_exists() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        std::env::set_current_dir(dir).unwrap();

        let path = Path::new("exists.txt");
        std::fs::write(path, "").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        exists `exists.txt` → `/path/to/directory/exists.txt`
         - `/path/to/directory`
             └── `exists.txt` file [✅ read, ✅ write, ❌ execute]
        🛑
        ")
    }

    #[test]
    fn test_symlink_to_file() {
        // Use two separate temp directories to guarantee different paths on all platforms
        let target_temp = tempfile::tempdir().unwrap();
        let link_temp = tempfile::tempdir().unwrap();
        let target_dir = target_temp.path().canonicalize().unwrap();
        let link_dir = link_temp.path().canonicalize().unwrap();

        // Create target file in first tempdir
        let target_file = target_dir.join("target.txt");
        std::fs::write(&target_file, "content").unwrap();

        // Create symlink in second tempdir pointing to first tempdir
        let symlink_path = link_dir.join("link_to_target.txt");
        symlink_file(&target_file, &symlink_path).unwrap();

        let output = PathFacts::new(&symlink_path)
            .to_string()
            .replace(&target_dir.display().to_string(), "/path/to/target")
            .replace(&link_dir.display().to_string(), "/path/to/link")
            .replace('\\', "/")
            + "🛑";

        insta::assert_snapshot!(
            output,
            @r"
        exists `/path/to/link/link_to_target.txt` → `/path/to/target/target.txt`
         - Symlink target: `/path/to/target/target.txt`
         - `/path/to/link`
             └── `link_to_target.txt` file [✅ read, ✅ write, ❌ execute]
        🛑
        ");
    }

    #[test]
    fn test_symlink_to_directory() {
        // Use two separate temp directories to guarantee different paths on all platforms
        let target_temp = tempfile::tempdir().unwrap();
        let link_temp = tempfile::tempdir().unwrap();

        let target_dir = target_temp.path().canonicalize().unwrap();
        let link_dir = link_temp.path().canonicalize().unwrap();

        // Create target directory in first tempdir
        let target = target_dir.join("target_dir");
        std::fs::create_dir(&target).unwrap();

        // Create symlink in second tempdir pointing to first tempdir
        let symlink_path = link_dir.join("link_to_dir");
        symlink_dir(&target, &symlink_path).unwrap();

        let output = PathFacts::new(&symlink_path)
            .to_string()
            .replace(&target_dir.display().to_string(), "/path/to/target")
            .replace(&link_dir.display().to_string(), "/path/to/link")
            .replace('\\', "/")
            + "🛑";

        insta::assert_snapshot!(
            output,
            @r"
        exists `/path/to/link/link_to_dir` → `/path/to/target/target_dir`
         - Symlink target: `/path/to/target/target_dir`
         - `/path/to/link`
             └── `link_to_dir` directory [✅ read, ✅ write, ✅ execute]
        🛑
        ");
    }

    #[test]
    // Unix-only test, though the failure it covers is not unix-only. `CannotReadCWD`
    // fires whenever `std::env::current_dir()` fails, which can happen on Windows too
    // (e.g. the working directory lived on a removable or network drive that went
    // away, or its permissions were revoked out from under the process).
    //
    // What differs is how to *provoke* it deterministically in a test. On unix we
    // just delete the CWD while sitting in it. Windows holds an open handle to the
    // CWD and refuses to remove it (`remove_dir` returns an error), so that trick is
    // unavailable, and the remaining triggers (yanking a drive, racing an ACL change)
    // can't be staged reliably from a unit test. Hence unix-only exercises the path.
    #[cfg(unix)]
    fn test_cannot_read_cwd() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        std::env::set_current_dir(&dir).unwrap();

        // Remove the current working directory while we're still in it
        std::fs::remove_dir(&dir).unwrap();

        insta::assert_snapshot!(
            PathFacts::new("relative_path.txt")
                .to_string()
                .replace(
                    &std::fs::read_to_string(dir).unwrap_err().to_string(),
                    "{error}"
                ) + "🛑",
            @r"
        `relative_path.txt`
         - Cannot read current working directory: {error}
        🛑
        ");
    }

    // Root detection is "the path has no lexical parent". The spelling of a root
    // differs by platform, so each OS asserts its own: unix's `/` here, Windows's
    // `C:\` in `test_is_root_windows`. On Windows `/` is root-but-relative (it has
    // a RootDir component but no drive prefix), so it would not report as root.
    #[test]
    #[cfg(unix)]
    fn test_is_root() {
        insta::assert_snapshot!(
            PathFacts::new("/").to_string() + "🛑",
            @r"
        is root `/`
        🛑
        "
        );
    }

    // Windows analog of `test_is_root`. A drive root like `C:\` has a Prefix and a
    // RootDir but no further components, so it has no lexical parent and reports as
    // root, the same property `/` has on unix.
    #[test]
    #[cfg(windows)]
    fn test_is_root_windows() {
        insta::assert_snapshot!(
            PathFacts::new(r"C:\").to_string() + "🛑",
            @r"
        is root `C:\`
        🛑
        "
        );
    }

    #[test]
    fn test_prior_dir_problem_relative_path() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        std::env::set_current_dir(&dir).unwrap();

        insta::assert_snapshot!(
            // Create a relative path where the parent directories don't exist
            PathFacts::new(Path::new("a/b/c/does_not_exist.txt"))
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        cannot access `a/b/c/does_not_exist.txt` → `/path/to/directory/a/b/c/does_not_exist.txt`
         - Prior directory does not exist `/path/to/directory/a`
            - Missing `a` from parent directory:
              `/path/to/directory`
                 └── (empty)
        🛑
        ");
    }

    #[test]
    fn test_parent_directory_missing_write_permissions() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let readonly_dir = dir.join("readonly_dir");
        std::fs::create_dir(&readonly_dir).unwrap();

        set_read_only(&readonly_dir).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(readonly_dir.join("does_not_exist.txt"))
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        does not exist `/path/to/directory/readonly_dir/does_not_exist.txt`
         - Parent directory is missing write permissions (cannot create, delete, or modify files)
         - Missing `does_not_exist.txt` from parent directory:
           `/path/to/directory/readonly_dir` [✅ read, ❌ write, ✅ execute]
              └── (empty)
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_canonicalize_circular_symlink_absolute() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let link1 = dir.join("link1");
        let link2 = dir.join("link2");

        // Create circular symlinks
        symlink_file(&link2, &link1).unwrap();
        symlink_file(&link1, &link2).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(&link1)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize(&link1).unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `/path/to/directory/link1`
         - Cannot canonicalize due to error `{error}`
         - `/path/to/directory`
             ├── `link1` (exists)
             └── `link2`
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_canonicalize_circular_symlink_relative() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        std::env::set_current_dir(dir).unwrap();

        // Create circular symlinks with relative paths
        symlink_file("link2", "link1").unwrap();
        symlink_file("link1", "link2").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(Path::new("link1"))
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize("link1").unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `link1` → `/path/to/directory/link1`
         - Cannot canonicalize due to error `{error}`
         - `/path/to/directory`
             ├── `link1` (exists)
             └── `link2`
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_canonicalize_broken_symlink_absolute() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let broken_link = dir.join("broken_link");
        let nonexistent = dir.join("does_not_exist");

        // Create a symlink pointing to a non-existent target
        symlink_file(&nonexistent, &broken_link).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(&broken_link)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize(&broken_link).unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `/path/to/directory/broken_link`
         - Cannot canonicalize due to error `{error}`
         - `/path/to/directory`
             └── `broken_link` (exists)
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_canonicalize_broken_symlink_relative() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        std::env::set_current_dir(dir).unwrap();

        // Create a symlink pointing to a non-existent target (relative path)
        symlink_file("does_not_exist", "broken_link").unwrap();

        insta::assert_snapshot!(
            PathFacts::new(Path::new("broken_link"))
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize("broken_link").unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `broken_link` → `/path/to/directory/broken_link`
         - Cannot canonicalize due to error `{error}`
         - `/path/to/directory`
             └── `broken_link` (exists)
        🛑
        "
        );
    }

    #[test]
    // Unix-only: relies on POSIX mode bits via `PermissionsExt::set_mode` (here
    // dropping directory execute), which does not exist on Windows.
    #[cfg(unix)]
    fn test_cannot_canonicalize_no_execute_dir_with_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let no_exec_dir = dir.join("no_exec_dir");
        std::fs::create_dir(&no_exec_dir).unwrap();

        let file = no_exec_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        // Remove execute permission from directory (can read dir but not traverse)
        set_mode(&no_exec_dir, 0o644).unwrap(); // read + write, no execute

        insta::assert_snapshot!(
            PathFacts::new(&file)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace(&std::fs::canonicalize(&file).unwrap_err().to_string(), "{error}") + "🛑",
            @r"
        exists `/path/to/directory/no_exec_dir/file.txt`
         - Cannot canonicalize due to error `{error}`
         - `/path/to/directory/no_exec_dir` [✅ read, ✅ write, ❌ execute]
             └── `file.txt` (exists)
        🛑
        "
        );
    }

    #[test]
    // Unix-only: relies on POSIX mode bits via `PermissionsExt::set_mode` (here
    // dropping directory write and execute), which does not exist on Windows.
    #[cfg(unix)]
    fn test_cannot_canonicalize_no_write_dir_with_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let no_write_dir = dir.join("no_write_dir");
        std::fs::create_dir(&no_write_dir).unwrap();

        let file = no_write_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        // read only: no write (fires warning), no execute (canonicalize fails)
        set_mode(&no_write_dir, 0o444).unwrap();

        let output = PathFacts::new(&file)
            .to_string()
            .replace(&dir.display().to_string(), "/path/to/directory")
            .replace(
                &std::fs::canonicalize(&file).unwrap_err().to_string(),
                "{error}",
            )
            + "🛑";

        // Restore permissions so the tempdir can be cleaned up.
        set_mode(&no_write_dir, 0o755).unwrap();

        insta::assert_snapshot!(
            output,
            @r"
        exists `/path/to/directory/no_write_dir/file.txt`
         - Cannot canonicalize due to error `{error}`
         - Parent directory is missing write permissions (cannot create, delete, or modify files)
         - `/path/to/directory/no_write_dir` [✅ read, ❌ write, ❌ execute]
             └── `file.txt` (exists)
        🛑
        "
        );
    }

    #[test]
    // Unix-only: builds `AbsPath` from unix-rooted paths (`/` and `/pretend/...`),
    // which are not valid absolute paths on Windows (roots look like `C:\`).
    #[cfg(unix)]
    fn test_cannot_canonicalize_anything() {
        let path = PathBuf::from(r"/pretend/root/does/not/exist/somehow");
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "simulated error");
        let output = PathFacts {
            path: path.clone(),
            state: Err(Box::new(UnknownPath::CannotCanonicalizeAnything(
                CannotCanonicalizeAnything {
                    original: AbsPath::new(&path).unwrap(),
                    root: AbsPath::new(Path::new("/")).unwrap(),
                    root_error: error,
                },
            ))),
        }
        .to_string()
            + "🛑";

        insta::assert_snapshot!(
            output,
            @r"
        `/pretend/root/does/not/exist/somehow`
         - Cannot canonicalize root `/` due to error: simulated error
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_metadata_exists() {
        // `CannotMetadata` is only reachable at runtime via a TOCTOU race, so we
        // construct the error state directly to exercise the Display branch.
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let file = dir.join("exists.txt");
        std::fs::write(&file, "").unwrap();

        let absolute = AbsPath::new(&file).unwrap();
        let parent = DirOk::new(absolute.lex_parent().unwrap()).unwrap();
        let canonical = CanonicalPath::new(&absolute).unwrap();
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "simulated");

        let facts = PathFacts {
            path: file.clone(),
            state: Err(Box::new(UnknownPath::CannotMetadata {
                absolute,
                canonical,
                parent,
                error,
            })),
        };

        insta::assert_snapshot!(
            facts
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        exists `/path/to/directory/exists.txt`
         - Cannot read metadata due to error `simulated`
         - `/path/to/directory`
             └── `exists.txt` (exists)
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_read_link_exists() {
        // `CannotReadLink` is only reachable at runtime via a TOCTOU race, so we
        // construct the error state directly to exercise the Display branch.
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let file = dir.join("exists.txt");
        std::fs::write(&file, "").unwrap();

        let absolute = AbsPath::new(&file).unwrap();
        let parent = DirOk::new(absolute.lex_parent().unwrap()).unwrap();
        let canonical = CanonicalPath::new(&absolute).unwrap();
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "simulated");

        let facts = PathFacts {
            path: file.clone(),
            state: Err(Box::new(UnknownPath::CannotReadLink {
                absolute,
                canonical,
                parent,
                error,
            })),
        };

        insta::assert_snapshot!(
            facts
                .to_string()
                .replace(&tempdir.path().canonicalize().unwrap().display().to_string(), "/path/to/directory")
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        exists `/path/to/directory/exists.txt`
         - Cannot readlink due to error `simulated`
         - `/path/to/directory`
             └── `exists.txt` (exists)
        🛑
        "
        );
    }
}

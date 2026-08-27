//! Facts about paths
use crate::Report;
use std::{fmt::Display, path::Path};

/// Shows helpful facts about a path when `Display`ed.
pub struct PathFacts {
    _inner: Report,
}

impl PathFacts {
    pub fn new(path: impl AsRef<Path>) -> Self {
        PathFacts {
            _inner: Report::new(path),
        }
    }
}

impl Display for PathFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self._inner.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abs_path::AbsPath;
    use crate::canonical_path::CannotCanonicalizeAnything;
    use crate::join_unfolded;
    use crate::test_support::*;
    use crate::trace::{CannotTrace, Trace};
    use std::path::PathBuf;

    // A `0o111` (search-only, no-read) parent directory: the walk can *search* through it to
    // resolve `child.txt`, so the path reaches its final component, but the parent cannot be
    // `read_dir`'d. That is a prior-path problem the trace cannot see (it records execute, not
    // read), so it is dispatched on `state == ParentProblem` and rendered by climbing to a
    // listable ancestor. Unix-only: search-without-read is a POSIX mode the Windows runner
    // cannot reproduce.
    #[test]
    #[cfg(unix)]
    fn test_prior_dir_problem_search_only_parent() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let search_only = dir.join("search_only");
        std::fs::create_dir(&search_only).unwrap();
        let child = search_only.join("child.txt");
        std::fs::write(&child, "").unwrap();

        // execute (searchable) but not readable
        set_mode(&search_only, 0o111).unwrap();

        let output = PathFacts::new(&child)
            .to_string()
            .replace(&dir.display().to_string(), "/path/to/directory")
            .replace('\\', "/")
            + "🛑";

        // Restore permissions so the tempdir can be cleaned up.
        set_mode(&search_only, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/search_only/child.txt`
         - Prior directory exists `/path/to/directory/search_only`
            - `/path/to/directory`
                └── `search_only` directory [❌ read, ❌ write, ✅ execute]
        🛑
        ");
    }

    #[test]
    fn test_prior_dir_problem_is_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let path = dir
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        let file = dir.join("a.txt");
        std::fs::write(&file, "").unwrap();

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
        does not exist `/path/to/directory/a/b/c/does_not_exist.txt`
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

        // `<dir>/a/b/..` resolves to the directory `<dir>/a`, so the resolution arrow points at
        // `<dir>/a` and the parent facts list `<dir>` annotating the `a` entry. Both the arrow
        // and the parent listing come from the `Trace`, which folds a trailing `..` left to
        // right. Sourcing them from `canonicalize` instead would drop the arrow on Windows,
        // where a verbatim (`\\?\`) path does not fold a `..`, and list the lexical parent
        // `<dir>/a/b` on every platform.
        let path = join_unfolded(&b, &[".."]);

        insta::assert_snapshot!(
            PathFacts::new(&path)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
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
            // Windows `canonicalize` yields a `\\?\` verbatim prefix, but `read_link` reports
            // the target without it. Strip the prefix everywhere so both forms match.
            .replace(r"\\?\", "")
            .replace(
                &target_dir.display().to_string().replace(r"\\?\", ""),
                "/path/to/target",
            )
            .replace(
                &link_dir.display().to_string().replace(r"\\?\", ""),
                "/path/to/link",
            )
            .replace('\\', "/")
            + "🛑";

        insta::assert_snapshot!(
            output,
            @r"
        exists `/path/to/link/link_to_target.txt` → `/path/to/target/target.txt`
         - Symlink → `/path/to/target/target.txt`
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
            // Windows `canonicalize` yields a `\\?\` verbatim prefix, but `read_link` reports
            // the target without it. Strip the prefix everywhere so both forms match.
            .replace(r"\\?\", "")
            .replace(
                &target_dir.display().to_string().replace(r"\\?\", ""),
                "/path/to/target",
            )
            .replace(
                &link_dir.display().to_string().replace(r"\\?\", ""),
                "/path/to/link",
            )
            .replace('\\', "/")
            + "🛑";

        insta::assert_snapshot!(
            output,
            @r"
        exists `/path/to/link/link_to_dir` → `/path/to/target/target_dir`
         - Symlink → `/path/to/target/target_dir`
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
        is root `C:\` → `\\?\C:\`
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
        does not exist `a/b/c/does_not_exist.txt` → `/path/to/directory/a/b/c/does_not_exist.txt`
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

        let output = PathFacts::new(readonly_dir.join("does_not_exist.txt"))
            .to_string()
            .replace(&dir.display().to_string(), "/path/to/directory")
            .replace('\\', "/")
            + "🛑";

        // Remove the deny-write ACE so the tempdir can be cleaned up.
        #[cfg(windows)]
        restore_write(&readonly_dir).unwrap();

        insta::assert_snapshot!(
            output,
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
                .replace(r"\\?\", "")
                .replace(&dir.display().to_string().replace(r"\\?\", ""), "/path/to/directory")
                .replace(&std::fs::canonicalize(&link1).unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `/path/to/directory/link1`
         - Symlink → `/path/to/directory/link2`
         - Cannot resolve target: {error}
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
                .replace(r"\\?\", "")
                .replace(
                    &tempdir.path().canonicalize().unwrap().display().to_string().replace(r"\\?\", ""),
                    "/path/to/directory",
                )
                .replace(&std::fs::canonicalize("link1").unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `link1` → `/path/to/directory/link1`
         - Symlink → `link2`
         - Absolute → `/path/to/directory/link2`
         - Cannot resolve target: {error}
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
                // Windows `canonicalize` yields a `\\?\` verbatim prefix, but `read_link`
                // reports the target without it. Strip the prefix everywhere so both forms match.
                .replace(r"\\?\", "")
                .replace(&dir.display().to_string().replace(r"\\?\", ""), "/path/to/directory")
                .replace(&std::fs::canonicalize(&broken_link).unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `/path/to/directory/broken_link`
         - Symlink → `/path/to/directory/does_not_exist`
         - Cannot resolve target: {error}
         - `/path/to/directory`
             └── `broken_link` (exists)
        🛑
        "
        );
    }

    #[test]
    fn test_broken_symlink_prior_absolute() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let broken_link = dir.join("broken_link");
        let nonexistent = dir.join("does_not_exist");

        // Create a symlink pointing to a non-existent target
        symlink_file(&nonexistent, &broken_link).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(broken_link.join("and").join("more.txt"))
                .to_string()
                .replace(r"\\?\", "")
                .replace(&dir.display().to_string().replace(r"\\?\", ""), "/path/to/directory")
                .replace(&std::fs::canonicalize(&broken_link).unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - Prior directory exists `/path/to/directory/broken_link`
            - Symlink → `/path/to/directory/does_not_exist`
            - Cannot resolve target: {error}
            - `/path/to/directory`
                └── `broken_link` (exists)
        🛑
        "
        );
    }

    #[test]
    fn test_broken_symlink_prior_relative() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();

        std::env::set_current_dir(&dir).unwrap();
        let broken_link = dir.join("broken_link");

        // Create a symlink pointing to a non-existent target
        symlink_file(Path::new("..").join("does_not_exist"), &broken_link).unwrap();

        insta::assert_snapshot!(
            PathFacts::new(broken_link.join("and").join("more.txt"))
                .to_string()
                .replace(r"\\?\", "")
                .replace(&dir.display().to_string().replace(r"\\?\", ""), "/path/to/directory")
                .replace(&std::fs::canonicalize(&broken_link).unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - Prior directory exists `/path/to/directory/broken_link`
            - Symlink → `../does_not_exist`
            - Absolute → `/path/to/directory/../does_not_exist`
            - Cannot resolve target: {error}
            - `/path/to/directory`
                └── `broken_link` (exists)
        🛑
        "
        );
    }

    /// A symlink-to-directory ancestor whose target holds a file that blocks descent. The
    /// early stop is reported at its physical location under the resolved target (`.../real/a.txt`),
    /// not under the link's own name, pinning that `step.at` is physical-through-parent and not a
    /// lexical spelling of the input.
    #[test]
    fn test_prior_path_is_file_under_a_symlinked_directory() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let real = dir.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("a.txt"), "").unwrap();
        symlink_dir(&real, dir.join("linkdir")).unwrap();

        let path = dir.join("linkdir").join("a.txt").join("b").join("x");

        insta::assert_snapshot!(
            PathFacts::new(path)
                .to_string()
                .replace(&dir.display().to_string(), "/path/to/directory")
                .replace('\\', "/") + "🛑",
            @r"
        does not exist `/path/to/directory/linkdir/a.txt/b/x`
         - Prior path is not a directory `/path/to/directory/real/a.txt`
            - `/path/to/directory/real`
                └── `a.txt` file [✅ read, ✅ write, ❌ execute]
        🛑
        ")
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
                // Windows `canonicalize` yields a `\\?\` verbatim prefix, but `read_link`
                // reports the target without it. Strip the prefix everywhere so both forms match.
                .replace(r"\\?\", "")
                .replace(
                    &tempdir.path().canonicalize().unwrap().display().to_string().replace(r"\\?\", ""),
                    "/path/to/directory",
                )
                .replace(&std::fs::canonicalize("broken_link").unwrap_err().to_string(), "{error}")
                .replace('\\', "/") + "🛑",
            @r"
        exists `broken_link` → `/path/to/directory/broken_link`
         - Symlink → `does_not_exist`
         - Absolute → `/path/to/directory/does_not_exist`
         - Cannot resolve target: {error}
         - `/path/to/directory`
             └── `broken_link` (exists)
        🛑
        "
        );
    }

    // Unix-only: the behavior is not reproducible on the Windows CI runner.
    //
    // The test needs "a directory you can list but not descend into" so that
    // canonicalizing a child fails. Unix models this as the directory execute bit,
    // dropped here with `set_mode(0o644)`.
    //
    // Windows models it as the "Traverse folder / execute file" ACL right. We tried
    // denying it with `icacls /deny <user>:(X)` (100% std via `std::process::Command`):
    // icacls reports success, but `canonicalize` still resolves the child. GitHub's
    // Windows runner executes as an administrator (`runneradmin`), and an admin token
    // bypasses per-user traverse denials, so the deny has no effect on the very process
    // running the test. Dropping that privilege mid-test is not something a unit test
    // can do reliably, so there is no way to provoke this state on the runner — with or
    // without std. Hence `#[cfg(unix)]`.
    #[test]
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
         - Cannot canonicalize due to error: {error}
         - `/path/to/directory/no_exec_dir` [✅ read, ✅ write, ❌ execute]
             └── `file.txt` (exists)
        🛑
        "
        );
    }

    // Unix-only for the same reason as `test_cannot_canonicalize_no_execute_dir_with_file`:
    // it removes directory traverse (0o444) so canonicalizing a child fails, and the
    // Windows CI runner is an administrator whose token bypasses a per-user traverse deny,
    // so `icacls /deny` (tried, 100% std) leaves canonicalize succeeding. Not reproducible
    // on the runner, hence `#[cfg(unix)]`.
    #[test]
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
         - Cannot canonicalize due to error: {error}
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
        let output = PathFacts {
            _inner: Report::from_trace_result(
                &path,
                Err(CannotTrace::RootNotReachable(CannotCanonicalizeAnything {
                    original: AbsPath::new(&path).unwrap(),
                    root: AbsPath::new(Path::new("/")).unwrap(),
                    root_error: std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "simulated error",
                    ),
                })),
            ),
        }
        .to_string()
            + "🛑";

        insta::assert_snapshot!(
            output,
            @r"
        `/pretend/root/does/not/exist/somehow`
         - Cannot canonicalize root `/` due to error (simulated error)
        🛑
        "
        );
    }

    /// A race caught on the final component. The walk resolved every step, then a re-check
    /// contradicted what it had just seen. Rendering names the race and still lists the parent,
    /// so the reader sees the entry that was there a moment ago. A real race cannot be timed on
    /// demand, so `Trace::inject_race` writes a known one into an otherwise real trace.
    #[test]
    fn test_renders_a_raced_final_component() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().canonicalize().unwrap();
        let path = dir.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        let mut trace = Trace::new(&path).unwrap();
        trace.append_race(
            "realpath resolved this link, stat failed to resolve",
            std::fs::metadata(dir.join("does_not_exist")).unwrap_err(),
        );

        let output = PathFacts {
            _inner: Report::from_trace_result(path.clone(), Ok(trace)),
        }
        .to_string()
        .replace(&dir.display().to_string(), "/path/to/directory")
        .replace('\\', "/")
            + "🛑";

        insta::assert_snapshot!(output, @r#"
        `/path/to/directory/a/b`
         - Race condition "realpath resolved this link, stat failed to resolve"  No such file or directory (os error 2)
         - Data race error: No such file or directory (os error 2)
           realpath resolved this link, stat failed to resolve.
         - `/path/to/directory/a`
             └── `b` (exists)
        🛑
        "#);
    }
}

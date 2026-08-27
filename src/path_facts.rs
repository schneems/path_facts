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

    /// Renders `PathFacts` and scrubs it with `scrubber`.
    fn facts(scrubber: &Scrubber, path: impl AsRef<Path>) -> String {
        scrubber.carets(&PathFacts::new(path).to_string())
    }

    // A `0o111` (search-only, no-read) parent directory: the walk can *search* through it to
    // resolve `child.txt`, so the path reaches its final component, but the parent cannot be
    // `read_dir`'d. That is a prior-path problem the trace cannot see (it records execute, not
    // read), so it is dispatched on `state == ParentProblem` and rendered by climbing to a
    // listable ancestor. Unix-only: search-without-read is a POSIX mode the Windows runner
    // cannot reproduce.
    #[test]
    #[cfg(unix)]
    fn test_prior_dir_problem_search_only_parent() {
        let fixture = Fixture::new();
        let search_only = fixture.join("search_only");
        std::fs::create_dir(&search_only).unwrap();
        let child = search_only.join("child.txt");
        std::fs::write(&child, "").unwrap();

        // execute (searchable) but not readable
        set_mode(&search_only, 0o111).unwrap();

        let output = facts(&fixture.scrub(), &child);

        // Restore permissions so the tempdir can be cleaned up.
        set_mode(&search_only, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/search_only/child.txt`
         - `/path/to/directory/search_only/child.txt`
                                           ^^^^^^^^^
                                           ↳ File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/search_only/child.txt`
                               ^^^^^^^^^^^
                               ↳ Dir [❌ read, ❌ write, ✅ execute]
                               ↳ Cannot list entries (no read permission)
                               ↳ Cannot create, delete, or rename entries (no write permission)
                               ↳ Cannot list directory: Permission denied (os error 13)
        🛑
        ");
    }

    #[test]
    fn test_prior_dir_problem_is_file() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();

        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::with_settings!({prepend_module_to_snapshot => false}, {
            insta::assert_snapshot!(
                "prior_dir_problem_is_file",
                facts(&fixture.scrub(), path)
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
        let fixture = Fixture::new();
        let path = fixture
            .join("a")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::assert_snapshot!(facts(&fixture.scrub(), path), @r"
        does not exist `/path/to/directory/a/b/c/does_not_exist.txt`
         - `/path/to/directory/a/b/c/does_not_exist.txt`
                               ^
                               ↳ Missing: No such file or directory (os error 2)
         - `/path/to/directory/a/b/c/does_not_exist.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `a`
                     ↳ Contains (0)
                       └── (empty)
        🛑
        ")
    }

    #[test]
    fn test_empty_path() {
        insta::assert_snapshot!(
            Scrubber::new().plain(&PathFacts::new("").to_string()),
            @r"
        path `` is empty
        🛑
        "
        )
    }

    #[test]
    fn test_file_exists_is_file() {
        let fixture = Fixture::new();
        let path = fixture.join("exists.txt");
        std::fs::write(&path, "").unwrap();

        insta::assert_snapshot!(facts(&fixture.scrub(), path), @r"
        exists `/path/to/directory/exists.txt`
         - `/path/to/directory/exists.txt`
                               ^^^^^^^^^^
                               ↳ File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/exists.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `exists.txt` (exists)
        🛑
        ")
    }

    #[test]
    fn test_exists_dot_dot_annotates_resolved_entry() {
        let fixture = Fixture::new();
        let b = fixture.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("inside.txt"), "").unwrap();
        std::fs::write(fixture.join("other.txt"), "").unwrap();

        // `<dir>/a/b/..` resolves to the directory `<dir>/a`, so the resolution arrow points at
        // `<dir>/a` and the parent facts list `<dir>` annotating the `a` entry. Both the arrow
        // and the parent listing come from the `Trace`, which folds a trailing `..` left to
        // right. Sourcing them from `canonicalize` instead would drop the arrow on Windows,
        // where a verbatim (`\\?\`) path does not fold a `..`, and list the lexical parent
        // `<dir>/a/b` on every platform.
        let path = join_unfolded(&b, &[".."]);

        insta::assert_snapshot!(facts(&fixture.scrub(), &path), @r"
        exists `/path/to/directory/a/b/..`
         - `/path/to/directory/a/b/..`
                                   ^^
                                   ↳ Dir [✅ read, ✅ write, ✅ execute]
                                   ↳ Canonical `/path/to/directory/a`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `a` (exists)
                       └── `other.txt`
        🛑
        ")
    }

    #[test]
    fn test_parent_exists_missing_file() {
        let fixture = Fixture::new();

        insta::assert_snapshot!(
            facts(&fixture.scrub(), fixture.join("does_not_exist.txt")),
            @r"
        does not exist `/path/to/directory/does_not_exist.txt`
         - `/path/to/directory/does_not_exist.txt`
                               ^^^^^^^^^^^^^^^^^^
                               ↳ Missing: No such file or directory (os error 2)
         - `/path/to/directory/does_not_exist.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `does_not_exist.txt`
                     ↳ Contains (0)
                       └── (empty)
        🛑
        ")
    }

    #[test]
    fn test_rename_two_missing_paths() {
        use indoc::formatdoc;

        let fixture = Fixture::new();
        fixture.enter();

        let from = Path::new("doesnotexist.txt");
        let to = Path::new("also_does_not_exist.txt");

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
                fixture.scrub().carets(&result.unwrap_err())
            );
        });
    }

    #[test]
    fn test_relative_path_exists() {
        let fixture = Fixture::new();
        fixture.enter();

        let path = Path::new("exists.txt");
        std::fs::write(path, "").unwrap();

        insta::assert_snapshot!(facts(&fixture.scrub(), path), @r"
        exists `exists.txt`
         - `exists.txt`
            ^^^^^^^^^^
            ↳ File [✅ read, ✅ write, ❌ execute]
            ↳ Absolute `/path/to/directory/exists.txt`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `exists.txt` (exists)
        🛑
        ")
    }

    #[test]
    fn test_symlink_to_file() {
        // Two fixtures guarantee the link and its target share no prefix on any platform.
        let target = Fixture::named("target");
        let link = Fixture::named("link");

        let target_file = target.join("target.txt");
        std::fs::write(&target_file, "content").unwrap();

        let symlink_path = link.join("link_to_target.txt");
        symlink_file(&target_file, &symlink_path).unwrap();

        let scrubber = link.scrub().path(target.anchor(), "");
        insta::assert_snapshot!(
            facts(&scrubber, &symlink_path),
            @r"
        exists `/path/to/link/link_to_target.txt`
         - `/path/to/link/link_to_target.txt`
                          ^^^^^^^^^^^^^^^^^^
                          ↳ Symlink [✅ read, ✅ write, ❌ execute]
                          ↳ Points to `/path/to/target/target.txt`
         - `/path/to/link/link_to_target.txt`
                     ^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `link_to_target.txt` (exists)
        🛑
        ");
    }

    #[test]
    fn test_symlink_to_directory() {
        // Two fixtures guarantee the link and its target share no prefix on any platform.
        let target = Fixture::named("target");
        let link = Fixture::named("link");

        let target_dir = target.join("target_dir");
        std::fs::create_dir(&target_dir).unwrap();

        let symlink_path = link.join("link_to_dir");
        symlink_dir(&target_dir, &symlink_path).unwrap();

        let scrubber = link.scrub().path(target.anchor(), "");
        insta::assert_snapshot!(
            facts(&scrubber, &symlink_path),
            @r"
        exists `/path/to/link/link_to_dir`
         - `/path/to/link/link_to_dir`
                          ^^^^^^^^^^^
                          ↳ Symlink [✅ read, ✅ write, ✅ execute]
                          ↳ Points to `/path/to/target/target_dir`
         - `/path/to/link/link_to_dir`
                     ^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `link_to_dir` (exists)
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
        let fixture = Fixture::new();
        fixture.enter();

        // Remove the current working directory while we're still in it
        let root = fixture.root().to_path_buf();
        std::fs::remove_dir(&root).unwrap();

        let scrubber = Scrubber::new().error(std::fs::read_to_string(&root).unwrap_err());
        insta::assert_snapshot!(facts(&scrubber, "relative_path.txt"), @r"
        `relative_path.txt`
         - `relative_path.txt`
            ↳ Cannot read the current directory: {error}
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
            Scrubber::new().plain(&PathFacts::new("/").to_string()),
            @r"
        is root `/`
        🛑
        "
        );
    }

    // Windows analog of `test_is_root`. A drive root like `C:\` has a Prefix and a
    // RootDir but no further components, so it has no lexical parent and reports as
    // root, the same property `/` has on unix. Unscrubbed: the backslashes and the
    // verbatim prefix are the point.
    #[test]
    #[cfg(windows)]
    fn test_is_root_windows() {
        insta::assert_snapshot!(
            format!("{}{STOP}", PathFacts::new(r"C:\")),
            @r"
        is root `C:\` → `\\?\C:\`
        🛑
        "
        );
    }

    #[test]
    fn test_prior_dir_problem_relative_path() {
        let fixture = Fixture::new();
        fixture.enter();

        insta::assert_snapshot!(
            // A relative path whose parent directories don't exist
            facts(&fixture.scrub(), "a/b/c/does_not_exist.txt"),
            @r"
        does not exist `a/b/c/does_not_exist.txt`
         - `a/b/c/does_not_exist.txt`
            ^
            ↳ Missing: No such file or directory (os error 2)
            ↳ Absolute `/path/to/directory/a/b/c/does_not_exist.txt`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `a`
                     ↳ Contains (0)
                       └── (empty)
        🛑
        ");
    }

    #[test]
    fn test_parent_directory_missing_write_permissions() {
        let fixture = Fixture::new();
        let readonly_dir = fixture.join("readonly_dir");
        std::fs::create_dir(&readonly_dir).unwrap();

        set_read_only(&readonly_dir).unwrap();

        let output = facts(&fixture.scrub(), readonly_dir.join("does_not_exist.txt"));

        // Remove the deny-write ACE so the tempdir can be cleaned up.
        #[cfg(windows)]
        restore_write(&readonly_dir).unwrap();

        insta::assert_snapshot!(
            output,
            @r"
        does not exist `/path/to/directory/readonly_dir/does_not_exist.txt`
         - `/path/to/directory/readonly_dir/does_not_exist.txt`
                                            ^^^^^^^^^^^^^^^^^^
                                            ↳ Missing: No such file or directory (os error 2)
         - `/path/to/directory/readonly_dir/does_not_exist.txt`
                               ^^^^^^^^^^^^
                               ↳ Dir [✅ read, ❌ write, ✅ execute]
                               ↳ Entries can be listed (read permission)
                               ↳ Cannot create, delete, or rename entries (no write permission)
                               ↳ ❌ Missing `does_not_exist.txt`
                               ↳ Contains (0)
                                 └── (empty)
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_canonicalize_circular_symlink_absolute() {
        let fixture = Fixture::new();
        let link1 = fixture.join("link1");
        let link2 = fixture.join("link2");

        // Create circular symlinks
        symlink_file(&link2, &link1).unwrap();
        symlink_file(&link1, &link2).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&link1).unwrap_err());
        insta::assert_snapshot!(
            facts(&scrubber, &link1),
            @r"
        exists `/path/to/directory/link1`
         - `/path/to/directory/link1`
                               ^^^^^
                               ↳ Symlink, unresolved
                               ↳ Points to `/path/to/directory/link2`
                               ↳ Cannot resolve target: {error}
         - `/path/to/directory/link1`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `link1` (exists)
                       └── `link2`
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_canonicalize_circular_symlink_relative() {
        let fixture = Fixture::new();
        fixture.enter();

        // Create circular symlinks with relative paths
        symlink_file("link2", "link1").unwrap();
        symlink_file("link1", "link2").unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize("link1").unwrap_err());
        insta::assert_snapshot!(
            facts(&scrubber, "link1"),
            @r"
        exists `link1`
         - `link1`
            ^^^^^
            ↳ Symlink, unresolved
            ↳ Points to `link2`
            ↳ Points to (absolute) `/path/to/directory/link2`
            ↳ Cannot resolve target: {error}
            ↳ Absolute `/path/to/directory/link1`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `link1` (exists)
                       └── `link2`
        🛑
        "
        );
    }

    #[test]
    fn test_cannot_canonicalize_broken_symlink_absolute() {
        let fixture = Fixture::new();
        let broken_link = fixture.join("broken_link");

        // Create a symlink pointing to a non-existent target
        symlink_file(fixture.join("does_not_exist"), &broken_link).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(
            facts(&scrubber, &broken_link),
            @r"
        exists `/path/to/directory/broken_link`
         - `/path/to/directory/broken_link`
                               ^^^^^^^^^^^
                               ↳ Symlink, unresolved
                               ↳ Points to `/path/to/directory/does_not_exist`
                               ↳ Cannot resolve target: {error}
         - `/path/to/directory/broken_link`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `broken_link` (exists)
        🛑
        "
        );
    }

    #[test]
    fn test_broken_symlink_prior_absolute() {
        let fixture = Fixture::new();
        let broken_link = fixture.join("broken_link");

        // Create a symlink pointing to a non-existent target
        symlink_file(fixture.join("does_not_exist"), &broken_link).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(
            facts(&scrubber, broken_link.join("and").join("more.txt")),
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - `/path/to/directory/broken_link/and/more.txt`
                               ^^^^^^^^^^^
                               ↳ Symlink, unresolved
                               ↳ Points to `/path/to/directory/does_not_exist`
                               ↳ Cannot resolve target: {error}
         - `/path/to/directory/broken_link/and/more.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `broken_link` (exists)
        🛑
        "
        );
    }

    #[test]
    fn test_broken_symlink_prior_relative() {
        let fixture = Fixture::new();
        fixture.enter();
        let broken_link = fixture.join("broken_link");

        // Create a symlink pointing to a non-existent target
        symlink_file(Path::new("..").join("does_not_exist"), &broken_link).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(
            facts(&scrubber, broken_link.join("and").join("more.txt")),
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - `/path/to/directory/broken_link/and/more.txt`
                               ^^^^^^^^^^^
                               ↳ Symlink, unresolved
                               ↳ Points to `../does_not_exist`
                               ↳ Points to (absolute) `/path/to/directory/../does_not_exist`
                               ↳ Cannot resolve target: {error}
         - `/path/to/directory/broken_link/and/more.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
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
        let fixture = Fixture::new();
        let real = fixture.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("a.txt"), "").unwrap();
        symlink_dir(&real, fixture.join("linkdir")).unwrap();

        let path = fixture.join("linkdir").join("a.txt").join("b").join("x");

        insta::assert_snapshot!(facts(&fixture.scrub(), path), @r"
        does not exist `/path/to/directory/linkdir/a.txt/b/x`
         - `/path/to/directory/linkdir/a.txt/b/x`
                                       ^^^^^
                                       ↳ File, not a dir [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/linkdir/a.txt/b/x`
                               ^^^^^^^
                               ↳ Dir [✅ read, ✅ write, ✅ execute]
                               ↳ Contains (1)
                                 └── `a.txt` (exists)
        🛑
        ")
    }

    #[test]
    fn test_cannot_canonicalize_broken_symlink_relative() {
        let fixture = Fixture::new();
        fixture.enter();

        // Create a symlink pointing to a non-existent target (relative path)
        symlink_file("does_not_exist", "broken_link").unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize("broken_link").unwrap_err());
        insta::assert_snapshot!(
            facts(&scrubber, "broken_link"),
            @r"
        exists `broken_link`
         - `broken_link`
            ^^^^^^^^^^^
            ↳ Symlink, unresolved
            ↳ Points to `does_not_exist`
            ↳ Points to (absolute) `/path/to/directory/does_not_exist`
            ↳ Cannot resolve target: {error}
            ↳ Absolute `/path/to/directory/broken_link`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `broken_link` (exists)
        🛑
        "
        );
    }

    // Unix-only: the behavior is not reproducible on the Windows CI runner.
    //
    // The test needs "a directory you can list but not descend into" so that the
    // lstat on a child is refused. Unix models this as the directory execute bit,
    // dropped here with `set_mode(0o644)`.
    //
    // Windows models it as the "Traverse folder / execute file" ACL right. We tried
    // denying it with `icacls /deny <user>:(X)` (100% std via `std::process::Command`):
    // icacls reports success, but the lstat still resolves the child. GitHub's
    // Windows runner executes as an administrator (`runneradmin`), and an admin token
    // bypasses per-user traverse denials, so the deny has no effect on the very process
    // running the test. Dropping that privilege mid-test is not something a unit test
    // can do reliably, so there is no way to provoke this state on the runner — with or
    // without std. Hence `#[cfg(unix)]`.
    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_no_execute_dir_with_file() {
        let fixture = Fixture::new();
        let no_exec_dir = fixture.join("no_exec_dir");
        std::fs::create_dir(&no_exec_dir).unwrap();

        let file = no_exec_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        // Remove execute permission from directory (can read dir but not traverse)
        set_mode(&no_exec_dir, 0o644).unwrap(); // read + write, no execute

        let scrubber = fixture
            .scrub()
            .error(std::fs::symlink_metadata(&file).unwrap_err());
        let output = facts(&scrubber, &file);

        // Restore permissions so the tempdir can be cleaned up.
        set_mode(&no_exec_dir, 0o755).unwrap();

        insta::assert_snapshot!(
            output,
            @r"
        exists `/path/to/directory/no_exec_dir/file.txt`
         - `/path/to/directory/no_exec_dir/file.txt`
                                           ^^^^^^^^
                                           ↳ Parent directory missing execute permission
                                           ↳ Cannot lstat: {error}
         - `/path/to/directory/no_exec_dir/file.txt`
                               ^^^^^^^^^^^
                               ↳ Dir [✅ read, ✅ write, ❌ execute]
                               ↳ Entries can be listed (read permission)
                               ↳ Cannot enter, traverse, or access entries (no execute permission)
                               ↳ Contains (1)
                                 └── `file.txt` (exists)
        🛑
        "
        );
    }

    // Unix-only for the same reason as `test_cannot_canonicalize_no_execute_dir_with_file`:
    // it removes directory traverse (0o444) so the lstat on a child is refused, and the
    // Windows CI runner is an administrator whose token bypasses a per-user traverse deny,
    // so `icacls /deny` (tried, 100% std) leaves the lstat succeeding. Not reproducible
    // on the runner, hence `#[cfg(unix)]`.
    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_no_write_dir_with_file() {
        let fixture = Fixture::new();
        let no_write_dir = fixture.join("no_write_dir");
        std::fs::create_dir(&no_write_dir).unwrap();

        let file = no_write_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        // read only: no write (fires warning), no execute (the lstat is refused)
        set_mode(&no_write_dir, 0o444).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::symlink_metadata(&file).unwrap_err());
        let output = facts(&scrubber, &file);

        // Restore permissions so the tempdir can be cleaned up.
        set_mode(&no_write_dir, 0o755).unwrap();

        insta::assert_snapshot!(
            output,
            @r"
        exists `/path/to/directory/no_write_dir/file.txt`
         - `/path/to/directory/no_write_dir/file.txt`
                                            ^^^^^^^^
                                            ↳ Parent directory missing execute permission
                                            ↳ Cannot lstat: {error}
         - `/path/to/directory/no_write_dir/file.txt`
                               ^^^^^^^^^^^^
                               ↳ Dir [✅ read, ❌ write, ❌ execute]
                               ↳ Entries can be listed (read permission)
                               ↳ Cannot create, delete, or rename entries (no write permission)
                               ↳ Cannot enter, traverse, or access entries (no execute permission)
                               ↳ Contains (1)
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
        .to_string();

        insta::assert_snapshot!(
            Scrubber::new().carets(&output),
            @r"
        `/pretend/root/does/not/exist/somehow`
         - `/pretend/root/does/not/exist/somehow`
                                         ^^^^^^^
                                         ↳ Root `/` is not reachable
                                         ↳ Cannot canonicalize: simulated error
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
        let fixture = Fixture::new();
        let path = fixture.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        let mut trace = Trace::new(&path).unwrap();
        trace.append_race(
            "realpath resolved this link, stat failed to resolve",
            std::fs::metadata(fixture.join("does_not_exist")).unwrap_err(),
        );

        let output = PathFacts {
            _inner: Report::from_trace_result(path.clone(), Ok(trace)),
        }
        .to_string();

        insta::assert_snapshot!(fixture.scrub().carets(&output), @r"
        `/path/to/directory/a/b`
         - `/path/to/directory/a/b`
                                 ^
                                 ↳ ⚠️ Filesystem change detected while gathering facts.
                                   ⚠️ Facts displayed may be invalid, stale or disagree.
                                   ⚠️
                                   ⚠️ Detected: realpath resolved this link, stat failed to resolve
                                   ⚠️ Error: No such file or directory (os error 2)
         - `/path/to/directory/a/b`
                               ^
                               ↳ Dir [✅ read, ✅ write, ✅ execute]
                               ↳ Contains (1)
                                 └── `b` (exists)
        🛑
        ");
    }
}

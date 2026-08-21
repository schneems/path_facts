//! A place to validate intuitions about paths. Tests
//! check std::fs behavior in addition to asserting interfaces
//! in this library.
#[cfg(test)]
mod tests {
    use crate::{abs_path::AbsPath, canonical_path::CanonicalPath, happy_path::DirOk};
    use std::path::{Component, Path};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    fn set_read_write_no_execute<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600); // Read and write for owner, no execute
        std::fs::set_permissions(path, permissions)?;
        Ok(())
    }

    #[cfg(unix)]
    fn set_write_execute_no_read<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o311); // Write and execute for owner, no read
        std::fs::set_permissions(path, permissions)?;
        Ok(())
    }

    #[cfg(unix)]
    fn set_read_write_execute<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)?;
        Ok(())
    }

    /// Asserts which error came back, not merely that one did
    ///
    /// The two tests below disagree on the kind for the same operation, so
    /// `is_err()` would let either of them pass the other's expectations.
    #[cfg(unix)]
    fn assert_err_kind<T: std::fmt::Debug>(
        result: std::io::Result<T>,
        expected: std::io::ErrorKind,
    ) {
        match result {
            Ok(value) => panic!("Expected `{:?}` error, got `Ok({:?})`", expected, value),
            Err(error) => assert_eq!(error.kind(), expected),
        }
    }

    /// Two spellings reach one directory. Comparing the paths cannot answer this, since a
    /// `..` or a symlink leaves them sharing no common prefix.
    #[cfg(unix)]
    fn is_same_dir(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;

        left.dev() == right.dev() && left.ino() == right.ino()
    }

    /// Directory missing execute means "cannot read metadata"
    ///
    /// The names of files in the directory can be listed but not traversed.
    /// i.e. `/dir/a/b/c` would not be reachable if `dir` is missing the execute
    /// permission, however we could see that it holds an `a` entry.
    ///
    /// Inverse of `test_dir_with_execute_without_read`.
    #[cfg(unix)]
    #[test]
    fn test_dir_without_execute() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let path = dir.join("exists.txt");
        std::fs::write(&path, "").unwrap();

        // The permission change below is the only thing that breaks the child
        assert!(std::fs::metadata(&path).is_ok());

        set_read_write_no_execute(dir).unwrap();

        let read_dir = std::fs::read_dir(dir).map(|_| ());
        let dir_ok = DirOk::new(AbsPath::new(dir).unwrap());
        let symlink_metadata = std::fs::symlink_metadata(&path);
        let canonical = CanonicalPath::new(&AbsPath::new(&path).unwrap());
        let contents = std::fs::read_to_string(&path);
        let metadata = std::fs::metadata(&path);
        let missing = std::fs::metadata(dir.join("does_not_exist.txt"));

        // Restore permissions so the tempdir can be cleaned up, a failed assertion
        // below would otherwise leave entries that cannot be unlinked.
        set_read_write_execute(dir).unwrap();

        // Can see the file, but cannot read it's metadata
        assert!(read_dir.is_ok());
        let dir_ok = dir_ok.unwrap();
        assert!(dir_ok.has_entry(&AbsPath::new(&path).unwrap()));

        assert_err_kind(symlink_metadata, std::io::ErrorKind::PermissionDenied);
        assert_err_kind(canonical, std::io::ErrorKind::PermissionDenied);
        assert_err_kind(contents, std::io::ErrorKind::PermissionDenied);
        assert_err_kind(metadata, std::io::ErrorKind::PermissionDenied);

        // A name that isn't there is denied rather than reported missing, the lookup is
        // refused before existence is decided. Listing is the only existence proof left.
        assert_err_kind(missing, std::io::ErrorKind::PermissionDenied);
    }

    // Directory missing read means "cannot list", but the children are still reachable
    //
    // i.e. `dir/a/b/c` could resolve if it exists however we could not list all entries in `dir`
    // to see `a` (or any other file).
    // Inverse of `test_dir_without_execute`. TODO: Improve output
    #[cfg(unix)]
    #[test]
    fn test_dir_with_execute_without_read() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("exec_no_read");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("child.txt");
        std::fs::write(&path, "hello").unwrap();

        set_write_execute_no_read(&dir).unwrap();

        let read_dir = std::fs::read_dir(&dir).map(|_| ());
        let dir_ok = DirOk::new(AbsPath::new(&dir).unwrap());
        let symlink_metadata = std::fs::symlink_metadata(&path);
        let canonical = CanonicalPath::new(&AbsPath::new(&path).unwrap());
        let contents = std::fs::read_to_string(&path);
        let missing = std::fs::metadata(dir.join("does_not_exist.txt"));

        // Restore permissions so the tempdir can be cleaned up, a failed assertion
        // below would otherwise leave a directory that cannot be listed to delete.
        set_read_write_execute(&dir).unwrap();

        // Cannot enumerate the names in the directory, so we cannot build a `DirOk`
        // and lose `has_entry` as a way to prove the child exists
        assert_err_kind(read_dir, std::io::ErrorKind::PermissionDenied);
        assert!(dir_ok.is_err());

        // Everything about the child is still available, including its contents
        assert!(symlink_metadata.is_ok());
        assert!(canonical.is_ok());
        assert_eq!(contents.unwrap(), "hello");

        // A name that isn't there is reported missing rather than denied, the opposite of
        // `test_dir_without_execute`. Lookup is the only existence proof left.
        assert_err_kind(missing, std::io::ErrorKind::NotFound);
    }

    /// `std::fs::canonicalize` can disagree with `std::fs::metadata`
    ///
    /// Both canonicalize and metadata "resolve" by following symlinks, it would seem that if you can
    /// get one then you should be guaranteed to be able to get the other with no error. However, that's
    /// not always the case. A real world example is that some OS's will enforce that `.` or `..` is
    /// a directory so if you have `a/dir/..` the `..` would "eat" `dir` but if you have `a/file.txt/..`
    /// the `..` will fail because `file.txt` is a file and not a dir.
    ///
    /// Canonicalize uses `realpath` while metadata uses `stat` (unix). Realpath is userspace,
    /// stat is kernel space and checks that a path part with something after it is a dir or will
    /// ENOTDIR. This is a exposed in Rust 1.83 as `ErrorKind::NotADirectory`
    #[cfg(target_vendor = "apple")]
    #[test]
    fn test_canonicalize_disagrees_with_metadata_on_a_trailing_dot_dot() {
        use crate::canonical_path::ExpandPath;

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let file = dir.join("file.txt");
        std::fs::write(&file, "hello").unwrap();

        let dotted = AbsPath::new(file.join("..")).unwrap();

        assert!(std::fs::metadata(dotted.as_ref()).is_err());
        assert!(std::fs::symlink_metadata(dotted.as_ref()).is_err());

        // Rust 1.83
        // assert_err_kind(
        //     std::fs::metadata(dotted.as_ref()),
        //     std::io::ErrorKind::NotADirectory,
        // );
        // assert_err_kind(
        //     std::fs::symlink_metadata(dotted.as_ref()),
        //     std::io::ErrorKind::NotADirectory,
        // );

        // Canonicalizing lands on `<dir>`, the answer the kernel refused to compute
        let canonical = CanonicalPath::new(&dotted).unwrap();
        assert_eq!(
            canonical,
            CanonicalPath::new(&AbsPath::new(dir).unwrap()).unwrap()
        );

        // Nothing is left over for `ExpandPath` to carry as a partial suffix
        assert_eq!(
            ExpandPath::new(&dotted)
                .unwrap()
                .canonical()
                .expect("whole path canonicalizes under Apple's libc"),
            canonical
        );

        // Canonical path answer diverges from input path
        std::fs::metadata(canonical.as_ref()).unwrap();
    }

    /// Posix: A trailing `..` reports as a dir (not a symlink) even when it points at a symlink
    /// Windows: A trailing `..` reports as a symlink because it is folded in
    ///          first before the check.
    ///          It's Path#is_dir() and symlink_metadat::is_dir() disagree
    #[test]
    fn test_symlink_metadata_for_a_path_ending_in_dot_dot() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let target = dir.join("x").join("y").join("z");
        std::fs::create_dir_all(&target).unwrap();
        let link = dir.join("link");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target, &link).unwrap();

        assert!(link.is_symlink());
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());

        let trailing = link.join("eaten").join("..");
        std::fs::create_dir_all(trailing.parent().unwrap()).unwrap();
        let meta = std::fs::symlink_metadata(&trailing).unwrap();

        let trailing_tuple = (trailing.is_symlink(), trailing.is_dir(), trailing.is_file());
        let meta_tuple = (meta.is_symlink(), meta.is_dir(), meta.is_file());
        #[cfg(unix)]
        {
            assert_eq!(trailing_tuple, (false, true, false));
            assert_eq!(meta_tuple, (false, true, false));
        }
        #[cfg(windows)]
        {
            assert_eq!(trailing_tuple, (true, true, false));
            // Diverges from Path::is_dir()
            assert_eq!(meta_tuple, (true, false, false));
        }

        // Check it would have reported as a symlink otherwiwse
        assert_eq!(
            trailing.canonicalize().unwrap(),
            target.canonicalize().unwrap()
        );
    }

    /// `Components` erases `.` and keeps `..`, which is why only one of them is safe
    ///
    /// A `.` needs no filesystem to interpret, so `Path` folds it away and is right to:
    /// `/a/b/.` reports a final `Normal("b")` and has the same parent as `/a/b`. A `..` is
    /// kept, and `parent` returns `/a/b` for `/a/b/..` — the entry the `..` cancels rather
    /// than any ancestor of the location.
    ///
    /// Every claim here is about `Path` alone, so nothing touches the disk and nothing is
    /// platform specific.
    #[test]
    fn test_components_erases_current_dir_but_keeps_parent_dir() {
        use std::ffi::OsStr;
        use std::path::Component;

        // `.` is gone from the component stream, trailing or interior
        assert_eq!(
            Path::new("/a/b/.").components().next_back(),
            Some(Component::Normal(OsStr::new("b")))
        );
        assert_eq!(
            Path::new("/a/./b").components().collect::<Vec<_>>(),
            Path::new("/a/b").components().collect::<Vec<_>>()
        );

        // `parent` drops a trailing `.` along with the name before it, so `X/.` and `X` agree
        assert_eq!(Path::new("/a/b/.").parent(), Path::new("/a/b").parent());
        assert_eq!(Path::new("/a/b/.").parent(), Some(Path::new("/a")));

        // An interior `.` survives in the bytes `parent` hands back, because the return is a
        // slice of the input rather than a rebuild from components. It takes `as_os_str` to
        // see that: `PartialEq` runs through `components`, which is blind to the difference.
        assert_eq!(
            Path::new("/a/./b/c").parent().unwrap().as_os_str(),
            OsStr::new("/a/./b")
        );
        assert_eq!(Path::new("/a/./b"), Path::new("/a/b"));

        // `..` is kept, and `parent` cancels it against the name before it. `/a/b/..` is the
        // location `/a`, whose parent is `/`, so `/a/b` is not an ancestor of anything here.
        assert_eq!(
            Path::new("/a/b/..").components().next_back(),
            Some(Component::ParentDir)
        );
        assert_eq!(Path::new("/a/b/..").parent(), Some(Path::new("/a/b")));
    }

    /// `Path::join` folds a `..` away when the receiver has a verbatim (`\\?\`)
    /// prefix, but appends it literally on any non-verbatim receiver.
    ///
    /// > if `self` has a verbatim prefix (e.g. `\\?\C:\windows`) and `path` is not
    /// > empty, the new path is normalized: all references to `.` and `..` are
    /// > removed.
    ///
    /// <https://doc.rust-lang.org/std/path/struct.PathBuf.html#method.push>
    ///
    /// `\\?\` prefixes only exist on Windows, so the verbatim half is
    /// `#[cfg(windows)]`; the non-verbatim half holds on every platform.
    #[test]
    fn test_path_join_folds_parent_dir_only_on_a_verbatim_receiver() {
        // Non-verbatim receiver: the `..` survives as a real component. Uses a
        // relative base so the assertion is identical on every platform.
        let plain = Path::new("base").join("a").join("b").join("..").join("c");
        assert!(
            plain
                .components()
                .any(|component| matches!(component, Component::ParentDir)),
            "join kept `..` on a non-verbatim base, got {:?}",
            plain
        );

        // Verbatim receiver: the `..` is folded at construction, so it never
        // reaches `components()`.
        #[cfg(windows)]
        {
            let verbatim = Path::new(r"\\?\C:\base\a\b").join("..").join("c");
            assert!(
                !verbatim
                    .components()
                    .any(|component| matches!(component, Component::ParentDir)),
                "join folded `..` on a verbatim base, got {:?}",
                verbatim
            );
            assert_eq!(verbatim, std::path::PathBuf::from(r"\\?\C:\base\a\c"));

            let literal = Path::new(r"\\?\C:\base\a\b\..\c");
            assert!(
                literal
                    .components()
                    .any(|component| matches!(component, Component::ParentDir)),
                "literal `..` on a verbatim base, got {:?}",
                literal
            );
        }
    }

    /// `create_dir_all("x/y/z/..")` creates `z`, even when `y` is missing
    ///
    /// Lexically `x/y/z/..` is `x/y`. `Path::parent` does not fold that way: the
    /// parent of `x/y/z/..` is `x/y/z`. `create_dir_all` walks that chain, so a
    /// missing `y` is filled by creating `x`, then `y`, then `z`. Only then does
    /// it `mkdir` the `..`. That name already exists as a directory entry, so the
    /// call returns `AlreadyExists`, and `is_dir("x/y/z/..")` is true because the
    /// path is `x/y`.
    ///
    /// The operation succeeds, and leaves behind a `z` that a folded reading of
    /// the path would never have asked for. `mkdir -p` does the same.
    #[cfg(unix)]
    #[test]
    fn test_create_dir_all_trailing_dot_dot_creates_the_cancelled_name() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let y = dir.join("x").join("y");
        let z = y.join("z");
        let dotted = z.join("..");

        assert_eq!(dotted.parent(), Some(z.as_path()));
        assert!(!y.exists());
        assert_err_kind(std::fs::create_dir(&dotted), std::io::ErrorKind::NotFound);

        std::fs::create_dir_all(&dotted).unwrap();

        assert!(y.is_dir());
        assert!(
            z.is_dir() && z.exists(),
            "`z` is created even though a trailing `..` cancels it"
        );
        assert!(is_same_dir(
            &std::fs::metadata(&dotted).unwrap(),
            &std::fs::metadata(&y).unwrap(),
        ));
    }
}

//! A place to validate intuitions about paths. Tests
//! check std::fs behavior in addition to asserting interfaces
//! in this library.
#[cfg(test)]
mod tests {
    use crate::{abs_path::AbsPath, canonical_path::CanonicalPath, happy_path::DirOk};
    use std::path::Path;

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

    /// `symlink_metadata` resolves a trailing `..`, it does not report on it
    ///
    /// `lstat` withholds resolution from a *symlink* in the final position and nothing else.
    /// A `..` is an ordinary entry naming the parent, so it resolves, and resolves
    /// physically: `<dir>/link/..` follows `link` first and lands beside the target.
    #[cfg(unix)]
    #[test]
    fn test_symlink_metadata_resolves_a_trailing_dot_dot() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        std::fs::create_dir_all(dir.join("x/y/z")).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(dir.join("x/y/z"), &link).unwrap();
        assert!(link.is_symlink());

        // `intermediate` is created through the link, so it sits in the target and `..`
        // lands back on the target
        let dotted = link.join("intermediate").join("..");
        std::fs::create_dir_all(dotted.parent().unwrap()).unwrap();
        let dotted_meta = std::fs::symlink_metadata(&dotted).unwrap();
        assert!(!dotted.is_symlink());
        assert!(dotted_meta.is_dir());

        // `lstat` on the link describes the link, `stat` describes the target
        assert!(!is_same_dir(
            &dotted_meta,
            &std::fs::symlink_metadata(&link).unwrap()
        ));
        assert!(is_same_dir(
            &dotted_meta,
            &std::fs::metadata(&link).unwrap()
        ));

        // `<dir>/link/..` is `<dir>/x/y`. A lexical reading would have said `<dir>`.
        let through_link = link.join("..");
        let through_meta = std::fs::symlink_metadata(&through_link).unwrap();
        assert!(!through_link.is_symlink());
        assert!(is_same_dir(
            &through_meta,
            &std::fs::symlink_metadata(dir.join("x/y")).unwrap()
        ));
        assert!(!is_same_dir(
            &through_meta,
            &std::fs::symlink_metadata(&dir).unwrap()
        ));

        // a path that ends in `..` cannot point a broken symlink because for `link/dir/..` to
        // be readable `link/dir` must exist, and if `link` is broken, it cannot. so symlink_metadata
        // there would fail with ErrKind::NotFound
    }

    /// Windows folds a trailing `..` after a symlink lexically, POSIX walks through the link
    ///
    /// This is the exact inverse of `test_symlink_metadata_resolves_a_trailing_dot_dot`. Same
    /// setup: `link` is a directory symlink to `x\y\z`, and we ask where `link\..` lands.
    ///
    /// - On POSIX the kernel follows `link` to `x/y/z` first, then applies `..`, landing on
    ///   `x/y` (proven by the unix test above).
    /// - On Windows the Win32 layer collapses `link\..` to its lexical parent, `<dir>`, as a
    ///   string operation before any I/O. The symlink is never followed. `symlink_metadata`
    ///   stats `<dir>`, which is not `x\y`.
    ///
    /// The consequence for this crate: a trailing `..` cannot be trusted to have traversed
    /// through the component it cancels, so the "read_link answering proves the last component
    /// is Normal" reasoning in `abs_path::readlink` does not hold on Windows.
    ///
    /// Creating a symlink on Windows needs SeCreateSymbolicLinkPrivilege (admin or Developer
    /// Mode). Without it the test cannot exercise the behavior, so it skips rather than
    /// falsely passing.
    #[cfg(windows)]
    #[test]
    fn test_windows_folds_a_trailing_dot_dot_after_a_symlink_lexically() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        let target = dir.join("x").join("y").join("z");
        std::fs::create_dir_all(&target).unwrap();
        let link = dir.join("link");

        std::os::windows::fs::symlink_dir(&target, &link).unwrap();
        assert!(link.is_symlink());

        // `link\..` folds to `<dir>` lexically, without following `link` to its target.
        let through_link = link.join("..");
        let through_meta = std::fs::symlink_metadata(&through_link).unwrap();
        assert!(!through_link.is_symlink());
        assert!(through_meta.is_dir());

        // Lands on `<dir>`, the lexical parent of `link` ...
        assert_eq!(
            std::fs::canonicalize(&through_link).unwrap(),
            std::fs::canonicalize(&dir).unwrap(),
        );
        // ... and specifically NOT on `x\y`, where the POSIX kernel would have landed.
        assert_ne!(
            std::fs::canonicalize(&through_link).unwrap(),
            std::fs::canonicalize(dir.join("x").join("y")).unwrap(),
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

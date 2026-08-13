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
            Ok(value) => panic!("Expected `{expected:?}` error, got `Ok({value:?})`"),
            Err(error) => assert_eq!(error.kind(), expected),
        }
    }

    // Directory missing execute means "cannot read metadata"
    //
    // The names of files in the directory can be listed but not traversed.
    // i.e. `/dir/a/b/c` would not be reachable if `dir` is missing the execute
    // permission, however we could see that it holds an `a` entry.
    //
    // Inverse of `test_dir_with_execute_without_read`.
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
}

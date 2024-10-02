//! A place to validate intuitions about paths. Tests
//! check std::fs behavior in addition to asserting interfaces
//! in this library.
#[cfg(test)]
mod tests {
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

    // Directory missing execute means "cannot read metadata"
    #[cfg(unix)]
    #[test]
    fn test_dir_without_execute() {
        use crate::{abs_path::AbsPath, happy_path::DirOk};

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let path = dir.join("exists.txt");
        std::fs::write(&path, "").unwrap();

        let result = std::fs::metadata(&path);
        assert!(result.is_ok());

        set_read_write_no_execute(dir).unwrap();
        let result = std::fs::metadata(&path);
        match result {
            Ok(_) => panic!("Expected error"),
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
            }
        }

        // Can see the file, but cannot read it's metadata
        let dir = DirOk::new(AbsPath::new(dir).unwrap()).unwrap();
        assert!(dir.has_entry(&AbsPath::new(&path).unwrap()));
    }
}

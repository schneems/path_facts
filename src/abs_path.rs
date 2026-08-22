//! An absolute path that may or may not exist on disk
//!
//! Holding this type guarantees that the path is not empty and the program has permission to read CWD.
//!
//! A property of absolute paths is that recursively retrieving their parent paths will eventually
//! lead to the root path. The parent of an absolute path is also an absolute path [`AbsPath::lex_parent`].
//!
//! If the held path is a readable directory, all children are also absolute paths [`AbsPath::read_dir`].
use std::{
    fmt::{Display, Formatter},
    path::{Component, Path, PathBuf, StripPrefixError},
};

/// Guaranteed to be relative
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RelativePath(PathBuf);

impl RelativePath {
    pub(crate) fn new(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref();
        if path.is_relative() {
            Some(Self(path.to_owned()))
        } else {
            None
        }
    }
}

impl AsRef<Path> for RelativePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbsPath(PathBuf);

impl AbsPath {
    pub(crate) fn new(path: impl AsRef<Path>) -> Result<Self, AbsPathError> {
        let path = path.as_ref();

        if path.as_os_str().is_empty() {
            return Err(AbsPathError::PathIsEmpty(path.to_owned()));
        }

        if path.is_relative() {
            let absolute = std::path::absolute(path)
                .map_err(|error| AbsPathError::CannotReadCWD(path.to_owned(), error))?;
            Ok(Self(absolute))
        } else {
            Ok(Self(path.to_owned()))
        }
    }

    /// Returns `self` expressed relative to `base`.
    ///
    /// - The returned relative path may be empty (when `self == base`).
    /// - Purely lexical: `..` is NOT resolved, so `/root/a/../b` and `/root/b` differ.
    /// - Errors if `base` is not a lexical prefix of `self`.
    pub(crate) fn strip_prefix(&self, base: &AbsPath) -> Result<RelativePath, StripPrefixError> {
        let diff = self.as_ref().strip_prefix(base.as_ref())?;
        Ok(RelativePath::new(diff).expect("path with stripped prefix is guaranteed relative"))
    }

    /// Tries to read the current path as a directory
    ///
    /// The properties of `read_dir` state that the resulting paths returned from `DirEntry`
    /// match the original path appended with the filename of the entry. Because we know
    /// the directory path is absolute, we know the resulting paths are absolute.
    ///
    /// Further this gives us the properties that calling `AbsPath::parent().read_dir()` should
    /// return a vector of paths that contain the original path if the original file exists. i.e.
    /// the format is the same.
    ///
    /// Errors if path is not a directory or is not readable
    pub fn join_relative(&self, path: &RelativePath) -> AbsPath {
        AbsPath(self.as_ref().join(path.as_ref()))
    }
    pub(crate) fn read_dir(&self) -> Result<Vec<AbsPath>, std::io::Error> {
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut entries: Vec<AbsPath> = std::fs::read_dir(&self.0)?
            .map(|entry| entry.map(|e| e.path()).map(AbsPath))
            .collect::<Result<Vec<AbsPath>, std::io::Error>>()?;

        // Sort by filename for deterministic test output only
        // In production, preserve the OS's native directory entry order
        #[cfg(test)]
        {
            entries.sort_by(|a, b| {
                let a_name = a.0.file_name().unwrap_or(a.0.as_os_str());
                let b_name = b.0.file_name().unwrap_or(b.0.as_os_str());
                a_name.cmp(b_name)
            });
        }

        Ok(entries)
    }

    /// Similar semantics to [`Path::parent`], but returning a None here would guarantee self is the root path
    ///
    /// An `AbsPath` is not normalized so it may contain `..` and/or symlinks. This is a lexical operation.
    ///
    /// ## Trailing ParentDir part (`..`)
    ///
    /// Not true for windows!
    ///
    /// A lexical parent might not be a physical ancestor when the last path is `..` i.e.
    ///
    /// ```text
    /// AbsPath::new("a/b/c/..").unwrap().lex_parent() -> Some("a/b/c")
    /// ```
    ///
    /// In this example, the lex_parent does not contain the child. The path `a/b/c/..` maps to the physical
    /// location of `a/b` therefore the physical parent would be `a`.
    ///
    /// The `..` ([`std::path::Component::ParentDir`]) can also interact with
    /// symlinks. If `a/b` is a symlink to `/x/y/z`, the kernel follows `a/b`
    /// to `/x/y/z`, so `a/b/c/..` is resolved as `/x/y/z/c/..`, which expands
    /// to `/x/y/z`. The physical parent would be `/x/y` and not `a`.
    ///
    /// ## ParentDir part (`..`) in the middle of a path
    ///
    /// Unlike a trailing `..`, one in the middle is not a hazard. When the last component is
    /// `Normal`, [`Path::parent`] hands back the prefix verbatim, `..` and all:
    ///
    /// ```text
    /// AbsPath::new("/a/b/../c/d").unwrap().lex_parent() -> Some("/a/b/../c")
    /// ```
    ///
    /// That result names the directory holding `d` (`a/b/c`). The kernel
    /// walks the same characters it walked for the original path, so it stops in the same
    /// place. Keeping the `..` unresolved is what makes this true: if `a/b` is a symlink to
    /// `/x/y/z`, then `/a/b/..` is `/x/y`, `d` lives in `/x/y/c`, and `/a/b/../c` resolves
    /// there too. Folding the `..` away first would give `/a/c`, a different directory that
    /// may not exist at all.
    ///
    /// ## CurrentDir in path
    ///
    /// A `.` in the middle is harmless. It survives in the prefix the same way
    /// (`/a/b/./c/d` -> `/a/b/./c`) and denotes the directory it appears to.
    ///
    /// It never survives at the *end* of a returned parent: [`Path::parent`] drops a trailing
    /// `.` along with the component before it, so `/a/b/./c` -> `/a/b` and `/a/b/.` -> `/a`.
    /// The second looks like it skips a level but is correct, because `/a/b/.` already denotes
    /// `/a/b`. Walking parents therefore visits each directory once, with no `/a/b/.` step in
    /// between. This is the opposite of the trailing `..` case above: `std` normalizes a
    /// trailing `.` and lands on the physical parent, and leaves a trailing `..` alone and
    /// does not.
    pub(crate) fn lex_parent(&self) -> Option<Self> {
        let parent = self.0.parent()?;

        Some(AbsPath(parent.to_path_buf()))
    }

    /// The same as `lex_parent` but will return root when trying to traverse beyond root
    pub(crate) fn lex_parent_or_root(&self) -> Self {
        self.lex_parent().unwrap_or_else(|| self.clone())
    }

    /// The filesystem root this path hangs off
    ///
    /// Lexical, like the rest of the `lex_` family: it reads the [`Component::Prefix`] and
    /// [`Component::RootDir`] parts off the front and asks the filesystem nothing. Every
    /// `AbsPath` has one, which is what being absolute means, so this cannot fail. A root
    /// is its own root.
    ///
    /// What comes back holds no `.` or `..` parts and no symlinks, which is normalized but
    /// not reachable. `\\server\share` names a machine that can be off. A caller that needs
    /// the root to *answer* has to canonicalize this and handle the failure.
    pub(crate) fn lex_root(&self) -> Self {
        let mut root = PathBuf::new();
        for component in self.0.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => root.push(component.as_os_str()),
                _ => break,
            }
        }
        AbsPath(root)
    }
}

impl Display for AbsPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`", self.0.display())
    }
}

impl AsRef<Path> for AbsPath {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

/// Reads where a symlink points
///
/// The caller must already know `absolute` is a symlink. [`std::fs::read_link`] answers
/// `InvalidInput` for anything that is not one, which arrives here indistinguishable from a
/// genuine failure, so a caller that has not checked cannot read this result.
///
/// A relative target resolves against the directory holding the symlink, not against the
/// symlink itself. A link at `/a/sub/rel` pointing at `gone` names `/a/sub/gone`.
///
/// The interface is wrong, it is displayed to the user such that it makes it seem that
/// an absolute path is written to the symlink (when relative). When in reality the relative
/// path can matter if the file is/was moved. TODO: Return (PathBuf, AbsPath) (or similar)
pub(crate) fn readlink(absolute: &AbsPath) -> Result<AbsPath, std::io::Error> {
    let target = std::fs::read_link(absolute.as_ref())?;

    if target.is_relative() {
        // `read_link` answering at all proves this path is a symlink, which proves its last
        // component is `Normal`: a trailing `..` or `.` resolves through whatever precedes
        // it and can never itself be a link. That is the condition `lex_parent` needs to be
        // read physically rather than lexically, per its own docs.
        //
        // This doesn't hold for windows, so this is incorrect on that platform
        let base = absolute.lex_parent_or_root();
        Ok(AbsPath(base.0.join(target)))
    } else {
        Ok(AbsPath(target))
    }
}

#[derive(Debug)]
pub(crate) enum AbsPathError {
    PathIsEmpty(PathBuf),
    CannotReadCWD(PathBuf, std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(path: impl AsRef<Path>) -> AbsPath {
        AbsPath::new(path).unwrap()
    }

    fn tempdir() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        (temp, dir)
    }

    /// Spelled through properties rather than through `/`, so the test says the same thing
    /// on a platform where a root is a drive letter or a share.
    #[test]
    fn test_lex_root_is_the_part_of_the_path_before_any_name() {
        let path = abs(tempdir().1.join("a").join("b"));

        let root = path.lex_root();

        assert!(path.as_ref().starts_with(root.as_ref()));
        assert!(root
            .as_ref()
            .components()
            .all(|part| matches!(part, Component::Prefix(_) | Component::RootDir)));
    }

    #[test]
    fn test_lex_root_of_a_root_is_itself() {
        let root = abs(tempdir().1).lex_root();

        assert_eq!(root.lex_root(), root);
    }

    /// Unlike `lex_parent`, nothing about this needs the path to be normalized: the parts it
    /// reads sit in front of anything that could be a `..` or a symlink.
    #[test]
    fn test_lex_root_ignores_the_rest_of_the_path() {
        let dir = tempdir().1;

        assert_eq!(
            abs(dir.join("a").join("..").join("b")).lex_root(),
            abs(&dir).lex_root()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_readlink_absolute_target_is_reported_verbatim() {
        let (_temp, dir) = tempdir();
        let symlink = dir.join("symlink");
        let target = dir.join("target");
        std::os::unix::fs::symlink(&target, &symlink).unwrap();

        let readlink = readlink(&abs(&symlink)).unwrap();
        assert_eq!(readlink.as_ref(), target);
    }

    #[cfg(unix)]
    #[test]
    fn test_readlink_relative_target_resolves_against_the_link_directory() {
        let (_temp, dir) = tempdir();
        let symlink = dir.join("symlink");
        std::fs::create_dir_all(symlink.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("target", &symlink).unwrap();

        let readlink = readlink(&abs(&symlink)).unwrap();
        assert_eq!(readlink.as_ref(), dir.join("target"));
    }

    /// Only symlinks have targets. A path that exists but is not a link, and a path that
    /// does not exist at all, are both "no target" rather than an error.
    #[test]
    fn test_readlink_without_a_symlink_is_none() {
        let (_temp, dir) = tempdir();
        std::fs::write(dir.join("f"), "").unwrap();

        assert!(readlink(&abs(dir.join("f"))).is_err());
        assert!(readlink(&abs(dir.join("missing"))).is_err());
    }
}

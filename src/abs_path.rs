//! Module for logic related toAn absolute path that may or may not exist on disk
use std::{
    fmt::{Display, Formatter},
    path::{Component, Path, PathBuf},
};

/// Holds an absolute path that has been normalized (no `..` or `.`)
///
/// - All guarantees from [`AbsPath`] hold
/// - Plus the expanded path is guaranteed to not escape root
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbsExpanded(AbsPath);

#[allow(dead_code)]
impl AbsExpanded {
    /// Normalize a path, including `..` without traversing the filesystem.
    ///
    /// Returns an error if normalization would leave leading `..` components.
    ///
    /// <div class="warning">
    ///
    /// This function always resolves `..` to the "lexical" parent.
    /// That is "a/b/../c" will always resolve to `a/c` which can change the meaning of the path.
    /// In particular, `a/c` and `a/b/../c` are distinct on many systems because `b` may be a symbolic link, so its parent isn't `a`.
    ///
    /// </div>
    ///
    /// [`path::absolute`](absolute) is an alternative that preserves `..`.
    /// Or [`Path::canonicalize`] can be used to resolve any `..` by querying the filesystem.
    /// implementation from: #[unstable(feature = "normalize_lexically", issue = "134694")]
    pub(crate) fn new(abs_path: AbsPath) -> Result<Self, AbsExpandedError> {
        let AbsPath(path) = &abs_path;
        let mut lexical = PathBuf::new();
        let mut iter = path.components().peekable();

        // Find the root, if any, and add it to the lexical path.
        // Here we treat the Windows path "C:\" as a single "root" even though
        // `components` splits it into two: (Prefix, RootDir).
        let root = match iter.peek() {
            Some(Component::ParentDir) => return Err(AbsExpandedError(abs_path)),
            Some(p @ Component::RootDir) | Some(p @ Component::CurDir) => {
                lexical.push(p);
                iter.next();
                lexical.as_os_str().len()
            }
            Some(Component::Prefix(prefix)) => {
                lexical.push(prefix.as_os_str());
                iter.next();
                if let Some(p @ Component::RootDir) = iter.peek() {
                    lexical.push(p);
                    iter.next();
                }
                lexical.as_os_str().len()
            }
            None => return Ok(AbsExpanded(abs_path)),
            Some(Component::Normal(_)) => 0,
        };

        for component in iter {
            match component {
                Component::RootDir => unreachable!(),
                Component::Prefix(_) => return Err(AbsExpandedError(abs_path)),
                Component::CurDir => continue,
                Component::ParentDir => {
                    // It's an error if ParentDir causes us to go above the "root".
                    if lexical.as_os_str().len() == root {
                        return Err(AbsExpandedError(abs_path));
                    } else {
                        lexical.pop();
                    }
                }
                Component::Normal(path) => lexical.push(path),
            }
        }
        Ok(AbsExpanded(
            AbsPath::new(lexical).expect("lexical absolute path must be an absolute path"),
        ))
    }

    pub(crate) fn read_dir(&self) -> Result<Vec<Self>, std::io::Error> {
        self.0.read_dir().map(|vec| {
            vec.into_iter()
                .map(|path| AbsExpanded::new(path).expect("inner path already expanded"))
                .collect()
        })
    }

    pub(crate) fn parent(&self) -> Option<Self> {
        self.0
            .parent()
            .map(|path| AbsExpanded::new(path).expect("inner path already expanded"))
    }
}

/// If a `..` parent reference would escape the path.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct AbsExpandedError(pub(crate) AbsPath);

impl Display for AbsExpanded {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`", self.0.as_ref().display())
    }
}

impl AsRef<Path> for AbsExpanded {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

/// Holds a reference to an (unresolved) absolute path
///
/// On unix if the path does not begin with a `/` then the absolute
/// path will be resolved with [`std::path::absolute`]. This method
/// does NOT resolve internal relative paths i.e. `/a/../b` is considered
/// a valid absolute path (even though there's a relative path part inside of it).
///
/// To resolve all internal relative paths (as well as symlinks) use [`crate::canonical_path::CanonicalPath`]
///
/// Holding this type guarantees that the path is not empty and the program has permission to read CWD.
///
/// A property of absolute paths is that recursively retrieving their parent paths will eventually
/// lead to the root path. The parent of an absolute path is also an absolute path [`AbsPath::parent`].
///
/// If the held path is a readable directory, all children are also absolute paths [`AbsPath::read_dir`].
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
            // std::path::absolute MAY check current_dir but is not guaranteed to do so (if the input
            // is already absolute) calling `current_dir()` adds an additional guarantee to the type
            // PROBLEM: A test modifes CWD to test edge cases in another thread, this code
            // now means basically every path fails randomly.
            // let _ = std::env::current_dir()
            //     .map_err(|error| AbsPathError::CannotReadCWD(path.to_owned(), error))?;
            Ok(Self(path.to_owned()))
        }
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
    pub(crate) fn parent(&self) -> Option<Self> {
        let parent = self.0.parent()?;

        Some(AbsPath(parent.to_path_buf()))
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

/// Returns Err if `read_link` fails
/// Returns Ok(None) if the path is not a symlink or if [`std::fs::symlink_metadata`] fails
/// Otherwise returns Ok(Some(AbsPath)) with the target of the symlink
pub(crate) fn try_readlink(absolute: &AbsExpanded) -> Result<Option<AbsPath>, std::io::Error> {
    let path = absolute.as_ref();
    if path.is_symlink() {
        std::fs::read_link(path)
            .map(|target| {
                if target.is_relative() {
                    AbsPath(absolute.as_ref().join(target))
                } else {
                    AbsPath(target)
                }
            })
            .map(Some)
    } else {
        Ok(None)
    }
}

#[derive(Debug)]
pub(crate) enum AbsPathError {
    PathIsEmpty(PathBuf),
    CannotReadCWD(PathBuf, std::io::Error),
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    fn abs(path: &str) -> AbsPath {
        AbsPath::new(PathBuf::from(path)).unwrap()
    }

    #[test]
    fn abs_path_does_not_normalize_dots() {
        let AbsPath(inner) = abs("/a/b/c/../d");
        assert_eq!(inner, PathBuf::from("/a/b/c/../d"));
    }

    #[test]
    fn abs_expanded_normalizes_dots() {
        let AbsExpanded(normalized) = AbsExpanded::new(abs("/a/b/c/../d")).unwrap();

        assert_eq!(normalized, abs("/a/b/d"));
    }

    #[test]
    fn abs_expanded_root() {
        let AbsExpanded(normalized) = AbsExpanded::new(abs("/")).unwrap();

        assert_eq!(normalized, abs("/"));
    }

    #[test]
    fn does_not_escape_root() {
        let result = AbsExpanded::new(abs("/.."));
        assert!(
            result.is_err(),
            "expected {:?} to be Err but it was not",
            result
        );
    }
}

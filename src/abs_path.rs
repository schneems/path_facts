//! Module for logic related toAn absolute path that may or may not exist on disk
use std::{
    fmt::{Display, Formatter},
    path::{Component, Path, PathBuf},
};

/// Holds an absolute path that has been normalized (no `..` or `.`)
///
/// - All guarantees from [`AbsRaw`] hold
/// - Plus any `..` and `.` components have been resolved lexically
///
/// A `..` that would escape root is clamped to root (a no-op), matching the
/// behavior of Ruby's `File.expand_path`. For example `/..` expands to `/` and
/// `/a/../../b` expands to `/b`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbsPath(AbsRaw);

#[allow(dead_code)]
impl AbsPath {
    pub(crate) fn from(path: &Path) -> Result<Self, AbsPathError> {
        AbsRaw::new(path).map(AbsPath::new)
    }

    /// Normalize a path, including `..` without traversing the filesystem.
    ///
    /// This is a purely lexical expansion that mirrors Ruby's `File.expand_path`
    /// for `.` and `..`: a `..` that would escape the root is clamped to the root
    /// rather than escaping it (e.g. `/..` -> `/`, `/a/../../b` -> `/b`).
    ///
    /// <div class="warning">
    ///
    /// This function always resolves `..` to the "lexical" parent.
    /// That is "a/b/../c" will always resolve to `a/c` which can change the meaning of the path.
    /// In particular, `a/c` and `a/b/../c` are distinct on many systems because `b` may be a symbolic link, so its parent isn't `a`.
    ///
    /// </div>
    ///
    /// Note: unlike Ruby, multiple leading slashes are collapsed to a single root
    /// (e.g. `////some/path` -> `/some/path`) because this reuses [`Path::components`],
    /// which normalizes a leading run of separators to a single [`Component::RootDir`].
    ///
    /// [`std::path::absolute`] is an alternative that preserves `..`.
    /// Or [`Path::canonicalize`] can be used to resolve any `..` by querying the filesystem.
    /// implementation adapted from: #[unstable(feature = "normalize_lexically", issue = "134694")]
    pub(crate) fn new(abs_path: AbsRaw) -> Self {
        let AbsRaw(path) = &abs_path;
        let mut lexical = PathBuf::new();
        let mut iter = path.components().peekable();

        // Find the root, if any, and add it to the lexical path.
        // Here we treat the Windows path "C:\" as a single "root" even though
        // `components` splits it into two: (Prefix, RootDir).
        let root = match iter.peek() {
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
            // A leading `Normal`/`ParentDir` means the path is relative (no root),
            // so `..` clamps at len 0. `None` (empty) is unreachable for a valid
            // `AbsRaw`, but we treat it the same rather than panicking.
            Some(Component::Normal(_)) | Some(Component::ParentDir) | None => 0,
        };

        for component in iter {
            match component {
                Component::RootDir | Component::Prefix(_) => unreachable!(),
                Component::CurDir => continue,
                Component::ParentDir => {
                    if lexical.as_os_str().len() == root {
                        // Pop nothing, keep root. Mirrors Ruby's `File.expand_path`
                    } else {
                        lexical.pop();
                    }
                }
                Component::Normal(path) => lexical.push(path),
            }
        }
        AbsPath(AbsRaw::new(lexical).expect("lexical absolute path must be an absolute path"))
    }

    pub(crate) fn read_dir(&self) -> Result<Vec<Self>, std::io::Error> {
        self.0
            .read_dir()
            .map(|vec| vec.into_iter().map(AbsPath::new).collect())
    }

    pub(crate) fn parent(&self) -> Option<Self> {
        self.0.parent().map(AbsPath::new)
    }
}

impl Display for AbsPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`", self.0.as_ref().display())
    }
}

impl AsRef<Path> for AbsPath {
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
/// Holding this type guarantees that the path is not empty.
///
/// A property of absolute paths is that recursively retrieving their parent paths will eventually
/// lead to the root path. The parent of an absolute path is also an absolute path [`AbsPath::parent`].
///
/// If the held path is a readable directory, all children are also absolute paths [`AbsPath::read_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbsRaw(PathBuf);

impl AbsRaw {
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
    pub(crate) fn read_dir(&self) -> Result<Vec<AbsRaw>, std::io::Error> {
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut entries: Vec<AbsRaw> = std::fs::read_dir(&self.0)?
            .map(|entry| entry.map(|e| e.path()).map(AbsRaw))
            .collect::<Result<Vec<AbsRaw>, std::io::Error>>()?;

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

        Some(AbsRaw(parent.to_path_buf()))
    }
}

impl Display for AbsRaw {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`", self.0.display())
    }
}

impl AsRef<Path> for AbsRaw {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

/// Returns Err if `read_link` fails
/// Returns Ok(None) if the path is not a symlink or if [`std::fs::symlink_metadata`] fails
/// Otherwise returns Ok(Some(AbsPath)) with the target of the symlink
pub(crate) fn try_readlink(absolute: &AbsPath) -> Result<Option<AbsPath>, std::io::Error> {
    let path = absolute.as_ref();
    if path.is_symlink() {
        std::fs::read_link(path)
            .map(|target| {
                if target.is_relative() {
                    AbsPath::from(&absolute.as_ref().join(target)).expect("absolute")
                } else {
                    AbsPath::from(&target).expect("absolute")
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

    fn abs(path: &str) -> AbsRaw {
        AbsRaw::new(PathBuf::from(path)).unwrap()
    }

    #[test]
    fn abs_path_does_not_normalize_dots() {
        let AbsRaw(inner) = abs("/a/b/c/../d");
        assert_eq!(inner, PathBuf::from("/a/b/c/../d"));
    }

    #[test]
    fn abs_expanded_normalizes_dots() {
        let AbsPath(normalized) = AbsPath::new(abs("/a/b/c/../d"));

        assert_eq!(normalized, abs("/a/b/d"));
    }

    #[test]
    fn abs_expanded_root() {
        let AbsPath(normalized) = AbsPath::new(abs("/"));

        assert_eq!(normalized, abs("/"));
    }

    #[test]
    fn parent_dir_at_root_clamps_to_root() {
        let AbsPath(normalized) = AbsPath::new(abs("/.."));

        assert_eq!(normalized, abs("/"));
    }

    #[test]
    fn parent_dir_past_root_clamps() {
        // Ruby: File.expand_path("../../bin", "/tmp/x") == "/bin"
        let AbsPath(normalized) = AbsPath::new(abs("/a/../../b"));
        assert_eq!(normalized, abs("/b"));

        // Ruby: File.expand_path('/tmp/../../../tmp') == '/tmp'
        let AbsPath(normalized) = AbsPath::new(abs("/tmp/../../../tmp"));
        assert_eq!(normalized, abs("/tmp"));
    }

    #[test]
    fn cur_dir_is_removed() {
        let AbsPath(normalized) = AbsPath::new(abs("/./dir"));
        assert_eq!(normalized, abs("/dir"));

        let AbsPath(normalized) = AbsPath::new(abs("/a/./b"));
        assert_eq!(normalized, abs("/a/b"));
    }

    #[test]
    fn only_exact_dot_and_dotdot_are_special() {
        // Names that merely start with dots are ordinary path elements,
        // matching Ruby's expand_path (and Rust's `Component` semantics).
        let AbsPath(normalized) = AbsPath::new(abs("/..a"));
        assert_eq!(normalized, abs("/..a"));

        let AbsPath(normalized) = AbsPath::new(abs("/a../b"));
        assert_eq!(normalized, abs("/a../b"));

        let AbsPath(normalized) = AbsPath::new(abs("/a."));
        assert_eq!(normalized, abs("/a."));
    }
}

#[cfg(test)]
#[cfg(windows)]
mod windows_tests {
    use super::*;

    fn abs(path: &str) -> AbsRaw {
        AbsRaw::new(PathBuf::from(path)).unwrap()
    }

    #[test]
    fn parent_dir_past_drive_root_clamps() {
        // Exercises the `Component::Prefix` + `RootDir` root-detection branch.
        let AbsPath(normalized) = AbsPath::new(abs(r"C:\a\..\..\b"));
        assert_eq!(normalized, abs(r"C:\b"));
    }

    #[test]
    fn parent_dir_at_drive_root_clamps_to_root() {
        let AbsPath(normalized) = AbsPath::new(abs(r"C:\.."));
        assert_eq!(normalized, abs(r"C:\"));
    }
}

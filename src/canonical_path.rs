//! If a path can be canonicalized it exists on disk and any symlinks in it's
//! path can be resolved to entities on disk.
//!
//! It can still have other problems, such as being a file when it's
//! expected to be a directory or not having correct permissions, but
//! we can guarantee that all files involved exist.
//!
//! Built from a [`AbsPath`] so we know the program has access to CWD.
use std::{
    fmt::Display,
    path::{Component, Path, PathBuf},
};

use crate::abs_path::{AbsPath, RelativePath};

/// Represents a partially cannonicalized path
#[derive(Debug, Clone)]
pub(crate) struct PartialCanonicalPath {
    /// Parent or prior directory that can be canonicalized
    pub(crate) prior: CanonicalPath,
    /// Rest of the path that could not be canonicalized, may have un-normalized parts i.e. `..`
    pub(crate) rest: RelativePath,
}

impl From<PartialCanonicalPath> for AbsPath {
    fn from(value: PartialCanonicalPath) -> Self {
        let PartialCanonicalPath { prior, rest } = value;
        AbsPath::new(prior.as_ref().to_owned().join(rest.as_ref()))
            .expect("canonical path is always absolute")
    }
}

/// Always contains a full or partial [`CanonicalPath`]
///
/// Any [`CanonicalPath`] exists, but we don't know other things about it without querying the
/// filesystem.
#[derive(Debug, Clone)]
pub(crate) enum ExpandPath {
    /// Path exists, fully expanded
    Canonical(CanonicalPath),
    /// Part of a path exists and can be expanded, the full path either doesn't exist or we don't
    /// have permission or there is a broken symlink somewhere. NOT lexically normalized, could
    /// contain `..` or `.` parts.
    Partial(PartialCanonicalPath),
}

impl From<ExpandPath> for AbsPath {
    fn from(value: ExpandPath) -> Self {
        match value {
            ExpandPath::Canonical(canonical_path) => canonical_path.into(),
            ExpandPath::Partial(partial_canonical_path) => partial_canonical_path.into(),
        }
    }
}

impl From<CanonicalPath> for ExpandPath {
    fn from(value: CanonicalPath) -> Self {
        ExpandPath::Canonical(value)
    }
}

impl From<PartialCanonicalPath> for ExpandPath {
    fn from(value: PartialCanonicalPath) -> Self {
        ExpandPath::Partial(value)
    }
}

#[derive(Debug)]
pub(crate) struct CannotCanonicalizeAnything {
    pub(crate) original: AbsPath,
    pub(crate) root: AbsPath,
    pub(crate) root_error: std::io::Error,
}

impl ExpandPath {
    pub(crate) fn new(abs_path: &AbsPath) -> Result<Self, CannotCanonicalizeAnything> {
        // Walk the original path, then each lexical ancestor up to root.
        let ancestors = std::iter::successors(Some(abs_path.clone()), AbsPath::parent);

        let mut last_failure = None;
        for path in ancestors.peekable() {
            match CanonicalPath::new(&path) {
                Ok(can_path) => {
                    let relative = abs_path
                        .strip_prefix(&path)
                        .expect("ancestor is a lexical prefix of the original path");

                    // Empty path when the two paths are the same
                    if relative.as_ref().as_os_str().is_empty() {
                        return Ok(ExpandPath::Canonical(can_path));
                    } else {
                        return Ok(ExpandPath::Partial(PartialCanonicalPath {
                            prior: can_path,
                            rest: relative,
                        }));
                    };
                }
                Err(io_error) => last_failure = Some((path, io_error)),
            }
        }

        let (root, root_error) =
            last_failure.expect("loop either returns or populates this value, loop guaranteed to have at least one value");
        Err(CannotCanonicalizeAnything {
            original: abs_path.clone(),
            root,
            root_error,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn canonical(self) -> Result<CanonicalPath, PartialCanonicalPath> {
        match self {
            ExpandPath::Canonical(canonical_path) => Ok(canonical_path),
            ExpandPath::Partial(partial_canonical_path) => Err(partial_canonical_path),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn partial(self) -> Result<PartialCanonicalPath, CanonicalPath> {
        match self {
            ExpandPath::Canonical(canonical_path) => Err(canonical_path),
            ExpandPath::Partial(partial_canonical_path) => Ok(partial_canonical_path),
        }
    }
}

/// Cleaned path
///
/// Guaranteed to have no `..` (parent parts) in the path. Guaranteed that SOME of the path exists
/// if the input was CanonicalPath then the output is the input. If the input was PartialCanonicalPath
/// then the root (and maybe more) is guaranteed to exist. Chained `..` parent paths cannot escape root
/// of disk drive.
///
/// Lexical normalization has a problem where `a/b/..` could point to `a/` but if `b` is a symlink to
/// `/usr/bin/which` then `a/b/..` would represent `/usr/bin`. The input type ExpandPath guarantees
/// that as much of the path that exists on disk (that we can access/know about) has been canonicalized
/// then whatever is left over either doesn't exist (in which case there is no symlink indirection problem)
/// or is a broken symlink, in which case I'm not sure if this is strictly safe to show to people.
///
/// We need to make sure that we can show a symlink problem with this representation OR ensure
/// that is shown in some other way.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NormalizedPath(PathBuf);

impl AsRef<Path> for NormalizedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl NormalizedPath {
    pub(crate) fn new(input: impl Into<ExpandPath>) -> Self {
        match input.into() {
            ExpandPath::Canonical(canonical_path) => {
                NormalizedPath(canonical_path.as_ref().to_owned())
            }
            ExpandPath::Partial(PartialCanonicalPath { prior, rest }) => {
                let CanonicalPath(mut lexical) = prior;
                for component in rest.as_ref().components() {
                    match component {
                        Component::RootDir => unreachable!("relative path does not have root dir"),
                        Component::Prefix(_) => unreachable!("relative path does not have prefix"),
                        Component::CurDir => {}
                        Component::ParentDir => match lexical.components().next_back() {
                            // Preserve
                            Some(Component::RootDir) => {}
                            // A real directory: cancel it out.
                            Some(Component::Normal(_)) => {
                                lexical.pop();
                            }
                            // Guarantteed path is not empty and first element does not start with `..`
                            None
                            // Stripped already
                            | Some(Component::CurDir)
                            // Stripped already
                            | Some(Component::ParentDir)
                            // prefix only exists when there's prefix + root and we're only ever looking one back
                            // we never remove a root
                            | Some(Component::Prefix(_)) => unreachable!(),
                        },
                        Component::Normal(p) => lexical.push(p),
                    }
                }

                NormalizedPath(lexical)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CanonicalPath(PathBuf);

impl From<CanonicalPath> for AbsPath {
    fn from(value: CanonicalPath) -> Self {
        AbsPath::new(value.0).unwrap()
    }
}

impl CanonicalPath {
    pub(crate) fn new(abs_path: &AbsPath) -> Result<Self, std::io::Error> {
        let canonical = abs_path.as_ref().canonicalize()?;
        Ok(CanonicalPath(canonical))
    }

    /// Similar semantics to [`AbsPath::parent`], but we guarantee return value
    /// exists and is normalized i.e. any CanonicalPath that is lexically equal is guaranteed
    /// to represent the same path on disk (TOCTOU caveat).
    ///
    /// A None here would guarantee self is the root path
    #[allow(dead_code)]
    pub(crate) fn parent(&self) -> Option<Self> {
        let parent = self.0.parent()?;

        Some(CanonicalPath(parent.to_path_buf()))
    }
}

impl AsRef<Path> for CanonicalPath {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl Display for CanonicalPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`", self.0.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prior_canonical() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let path = dir.join("does/not/exist.txt");
        let expand = ExpandPath::new(&AbsPath::new(path).unwrap()).unwrap();
        match expand {
            ExpandPath::Canonical(_) => panic!("expected partial got {:?}", expand),
            ExpandPath::Partial(PartialCanonicalPath { prior, rest }) => {
                assert_eq!(
                    prior,
                    CanonicalPath::new(&AbsPath::new(dir).unwrap()).unwrap()
                );
                assert_eq!(rest, RelativePath::new("does/not/exist.txt").unwrap());
            }
        }
    }

    #[test]
    fn test_canonical_expand() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let expand = ExpandPath::new(&AbsPath::new(dir).unwrap()).unwrap();

        match &expand {
            ExpandPath::Canonical(canonical_path) => assert_eq!(
                canonical_path,
                &CanonicalPath::new(&AbsPath::new(dir).unwrap()).unwrap()
            ),
            ExpandPath::Partial(PartialCanonicalPath { .. }) => {
                panic!("expected full got {:?}", expand)
            }
        }
    }

    fn expand(path: impl AsRef<Path>) -> ExpandPath {
        ExpandPath::new(&AbsPath::new(path).unwrap()).unwrap()
    }

    fn normal(path: impl AsRef<Path>) -> NormalizedPath {
        NormalizedPath::new(expand(path.as_ref()))
    }

    #[test]
    fn abs_expanded_normalizes_dots() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let normalized = normal(dir.join("a/b/c/../d"));

        assert_eq!(normalized, normal(dir.join("a/b/d")));
    }

    #[test]
    #[cfg(unix)]
    fn abs_expanded_root() {
        assert_eq!(normal("/").as_ref(), Path::new("/"));
        assert_eq!(normal("/..").as_ref(), Path::new("/"));
        assert_eq!(normal("/../..").as_ref(), Path::new("/"));
    }

    #[test]
    #[cfg(unix)]
    fn parent_dir_past_root_clamps() {
        assert_eq!(normal("/a/../b"), normal("/b"));
        assert_eq!(normal("/tmp/../../../tmp"), normal("/tmp"));
    }

    #[test]
    #[cfg(unix)]
    fn cur_dir_is_removed() {
        assert_eq!(normal("/./dir").as_ref(), Path::new("/dir"));
        assert_eq!(normal("/a/./b").as_ref(), Path::new("/a/b"));
    }

    #[test]
    #[cfg(unix)]
    fn only_exact_dot_and_dotdot_are_special() {
        // Names that merely start with dots are ordinary path elements,
        // matching Ruby's expand_path (and Rust's `Component` semantics).
        assert_eq!(normal("/..a").as_ref(), Path::new("/..a"));
        assert_eq!(normal("/..a/b").as_ref(), Path::new("/..a/b"));
        assert_eq!(normal("/a.").as_ref(), Path::new("/a."));
    }
}

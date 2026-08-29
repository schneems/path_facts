//! If a path can be canonicalized it exists on disk and any symlinks in it's
//! path can be resolved to entities on disk.
//!
//! It can still have other problems, such as being a file when it's
//! expected to be a directory or not having correct permissions, but
//! we can guarantee that all files involved exist.
//!
//! Built from a [`AbsPath`] so we know the program has access to CWD.
//! May have un-normalized parts i.e. `..`
use crate::{
    abs_path::{AbsPath, RelativePath},
    component::{self, NormalComponent},
};
use std::{
    fmt::Display,
    path::{Component, Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct CannotCanonicalizeAnything {
    pub(crate) original: AbsPath,
    pub(crate) root: AbsPath,
    pub(crate) root_error: std::io::Error,
}

/// What [`CanonicalPath::entry`] found at a name inside a directory
#[derive(Debug)]
pub(crate) enum Entry {
    /// Not a symlink, so the path is canonical and the metadata describes it directly
    Canonical(CanonicalPath, std::fs::Metadata),
    /// A symlink, which has to be followed before anything canonical can be said about it
    Symlink,
}

/// File exists, and is resolvable path, is fully normalized
///
/// - All symlinks resolve and are visible
///
/// Does not preserve behavior on all calls. Getting metadata from
/// `/a/b/file.txt/..` fails with NotADirectory on every unix, POSIX requires
/// ENOTDIR when a path prefix component is not a directory.
///
/// Canonicalizing it produces different results. Glibc enforces the same rule and errors, but a mac
/// returns `/a/b`. So on a mac this type gets built from a path the kernel refuses, and metadata on
/// the canonical form succeeds while metadata on the original still fails. A trailing `.` behaves the same way.
///
/// So using a `CanonicalPath` as a replacement for `Path` can yield subtle differences.
///
/// ## Windows
///
/// On windows when you canonicalize a path you get a UNC format. i.e. `C:\` → `\\?\C:\`.
/// This is considered a path literal where every element is resolved. A property of this
/// fact is that you cannot always canonicalize a path that's been canonicalized and modified
/// i.e. `path.canonicalize().join("..\other_path").canonicalize().unwrap()` will error
/// because `..` does not exist when the path is already using UNC format.
///
/// An alternative to canonicalizing and building UNC is in the dunce crate <https://gitlab.com/kornelski/dunce/-/blob/c523a1edfa81cd7603a28971154a33c14b2fed4e/src/lib.rs>
///
/// Also take care in tests that joining `".."` literal to a `Path` will fold it in.
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

    /// Joins input with the given Canonical path without checking disk contents
    ///
    /// Unsafe because the output is a CanonicalPath, so the caller must make sure that:
    ///
    /// - The path points to a resolved location on disk
    ///
    /// The location on disk either needs to be `lstat`-able or be observed
    /// in a directory (when the directory has read, but not execute permission).
    pub(crate) unsafe fn unchecked_join(&self, rest: &NormalComponent) -> CanonicalPath {
        CanonicalPath(self.as_ref().join(rest.as_ref()))
    }

    /// Resolves a relative path against this directory, folding the `..` parts it opens with
    ///
    /// Returns an [`AbsPath`] rather than a [`CanonicalPath`]: the fold answers where the relative
    /// path is anchored, and says nothing about whether the name it ends in is there. The main
    /// caller is a symlink's target, which is often exactly the name that is missing.
    ///
    /// Folding a leading `..` is sound for the same reason [`CanonicalPath::parent`] is: `self`
    /// holds no symlinks, so the lexical parent is the physical one, which is what POSIX resolves
    /// `..` to. Root is its own parent, matching `realpath` on `/..`.
    ///
    /// The fold stops at the first name, and it has to. A `..` *after* a name (`a/../b`) is a `..`
    /// out of `a`, which belongs to `rest` rather than to `self` and so was never shown to be a
    /// directory rather than a symlink somewhere else. Cancelling it would name a place the kernel
    /// would not go.
    ///
    /// Windows folds those too, in the [`Path::join`] below, because `self` is verbatim there and
    /// `push` normalizes onto a verbatim base. That matches the platform: Win32 collapses `..`
    /// lexically before the kernel sees a path at all.
    pub(crate) fn join_fold_leading_parent_dirs(&self, rest: &RelativePath) -> AbsPath {
        let mut base = self.clone();
        let mut rest = rest.as_ref().components().peekable();

        while let Some(component) = rest.peek() {
            match component {
                // A `.` denotes the directory it sits in, so there is nothing to fold
                Component::CurDir => {}
                Component::ParentDir => {
                    if let Some(parent) = base.parent() {
                        base = parent;
                    }
                }
                _ => break,
            }
            rest.next();
        }

        let rest = rest.map(Component::as_os_str).collect::<PathBuf>();
        let base = AbsPath::from(base);

        // A path of nothing but dot parts is folded away entirely, and appending the empty
        // remainder would leave a trailing separator on the directory it landed on.
        if rest.as_os_str().is_empty() {
            return base;
        }

        base.join_relative(
            &RelativePath::new(rest).expect("what follows the dot parts of a relative path"),
        )
    }

    /// Returns the filename of the path
    ///
    /// Since a canonical path is fully resolved, it will always be a normal component
    /// Returns None when the path is root (with no filename)
    pub(crate) fn filename_component(&self) -> Option<NormalComponent> {
        self.as_ref()
            .components()
            .next_back()
            .map(component::owned)
            .and_then(|component| component.normal().cloned())
    }

    /// Looks up `name` in this directory
    ///
    /// Takes its own `symlink_metadata` rather than accepting one. Metadata handed in by a
    /// caller says nothing about `name`, it could describe any path at all, so it cannot
    /// stand as proof of anything about the value returned here.
    ///
    /// A non-symlink entry is canonical without a `realpath` call, meeting each part of the
    /// contract on [`CanonicalPath`]:
    ///
    /// - Exists, because `symlink_metadata` succeeded on it.
    /// - Fully normalized, because `self` holds no symlinks and no `.` or `..` parts, and
    ///   `lstat` shows `name` is not a symlink. A `Normal` component adds no dot parts.
    /// - Every directory involved is executable, because `self` already carried that and
    ///   the lookup of `name` inside it just succeeded, which requires it.
    ///
    /// Which leaves resolvable: `realpath` would walk `self` (symlink free, searchable)
    /// and then find `name` present and not a link, so it has nothing left to resolve.
    ///
    /// Carries the same TOCTOU caveat as everything else in this library: all of the above
    /// was true when the syscall ran.
    pub(crate) fn entry(&self, name: &NormalComponent) -> Result<Entry, std::io::Error> {
        let path = self.0.join(name.as_ref());
        let lstat = std::fs::symlink_metadata(&path)?;

        if lstat.file_type().is_symlink() {
            Ok(Entry::Symlink)
        } else {
            Ok(Entry::Canonical(CanonicalPath(path), lstat))
        }
    }

    /// The names this directory holds, sorted
    ///
    /// Names rather than paths. "Is `x` in this directory" is the question every caller has, and a
    /// name answers it directly, while comparing two absolute paths also compares two spellings of
    /// the directory they hang off — a verbatim prefix on one side, or a symlink resolved on one
    /// side and not the other, reads as a file that is not there. Every call site already holds the
    /// directory, so nothing is lost by leaving it off.
    ///
    /// Sorted because `read_dir` hands back whatever order the filesystem stores names in, which is
    /// stable for nobody and comparable across nothing: APFS, ext4 and NTFS each answer
    /// differently for the same directory. A listing is read by a human next to an error message
    /// and diffed by a snapshot test, and both want the same list twice in a row.
    ///
    /// By name bytes, which is the order `cargo package --list` prints and the order `ls` prints
    /// under `LC_ALL=C`. It is not what `ls` prints otherwise: `ls` sorts by the locale's collating
    /// sequence, folding case and taking punctuation weights from a table that differs by platform
    /// and by the reader's environment. Reproducing that would mean carrying a collation table to
    /// produce a listing that changes with whoever reads it, so this library sorts the way the
    /// other developer-facing tool in the room does and says so rather than claiming to match `ls`.
    pub(crate) fn normal_entries(&self) -> Result<Vec<NormalComponent>, std::io::Error> {
        let mut entries: Vec<NormalComponent> = std::fs::read_dir(&self.0)?
            .map(|entry| entry.map(Into::<NormalComponent>::into))
            .collect::<Result<Vec<NormalComponent>, std::io::Error>>()?;

        entries.sort_by(|a, b| {
            AsRef::<std::ffi::OsStr>::as_ref(a).cmp(AsRef::<std::ffi::OsStr>::as_ref(b))
        });

        Ok(entries)
    }

    /// Similar semantics to [`AbsPath::lex_parent`], but we guarantee return value
    /// exists and is normalized i.e. any CanonicalPath that is lexically equal is guaranteed
    /// to represent the same path on disk (TOCTOU caveat).
    ///
    /// A None here would guarantee self is the root path
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

    fn canonical(path: &Path) -> CanonicalPath {
        CanonicalPath::new(&AbsPath::new(path).unwrap()).unwrap()
    }

    /// Relative paths are spelled with `/`, which Windows accepts, so both platforms see the same
    /// components. Expected values are built with `join` so the separators are the platform's.
    fn folded(dir: &CanonicalPath, rest: &str) -> PathBuf {
        dir.join_fold_leading_parent_dirs(&RelativePath::new(rest).unwrap())
            .as_ref()
            .to_path_buf()
    }

    /// Every leading dot part folds, not only the first, and a `.` is stepped over on the way.
    #[test]
    fn test_join_folded_consumes_the_whole_leading_run() {
        let temp = tempfile::tempdir().unwrap();
        let anchor = temp.path().canonicalize().unwrap();
        let b = anchor.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();
        let dir = canonical(&b);

        assert_eq!(folded(&dir, "x"), b.join("x"));
        assert_eq!(folded(&dir, "../x"), anchor.join("a").join("x"));
        assert_eq!(folded(&dir, "../../x"), anchor.join("x"));
        assert_eq!(folded(&dir, "./../x"), anchor.join("a").join("x"));

        // Consumed entirely, so the directory the fold landed on is the whole answer
        assert_eq!(folded(&dir, ".."), anchor.join("a"));
    }

    /// Root is its own parent, so a `..` run cannot walk off the top of the filesystem.
    #[test]
    fn test_join_folded_clamps_at_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .components()
            .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
            .map(|component| component.as_os_str())
            .collect::<PathBuf>();
        let dir = canonical(&root);

        assert_eq!(folded(&dir, "../../x"), dir.as_ref().join("x"));
    }

    /// A `..` after a name is a `..` out of that name, which belongs to the relative path rather
    /// than to this directory and was never shown to be a directory rather than a symlink
    /// somewhere else. So it is left for the kernel to resolve.
    ///
    /// Unix-only: Windows collapses those itself, in the `join` the fold ends with, which matches
    /// Win32 collapsing `..` lexically before the kernel sees a path.
    #[test]
    #[cfg(unix)]
    fn test_join_folded_leaves_a_dot_dot_after_a_name_alone() {
        let temp = tempfile::tempdir().unwrap();
        let dir = canonical(&temp.path().canonicalize().unwrap());

        assert_eq!(
            folded(&dir, "a/../x"),
            dir.as_ref().join("a").join("..").join("x")
        );
    }
}

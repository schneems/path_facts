//! Representations of paths with no problems
//!
//! Holds [`HappyPath`] and [`DirOk`]
use crate::{
    abs_path::{self, AbsPath, AbsPathError, RelativePath},
    canonical_path::{CannotCanonicalizeAnything, CanonicalPath, ExpandPath, PartialCanonicalPath},
    resolved_metadata::{ResolvedMetadata, ResolvedType},
};
use faccess::{AccessMode, PathExt};
use std::path::{Path, PathBuf};
use std::{fs::Metadata, path::Component};

// type Perms = (bool, bool, bool);

// // raw Component, from input
// enum PathNode<'a> {
//     CwdNode(CwdNode<'a>),
//     Input(InputNode<'a>),
// }

// type AbsVec<'a> = Vec<PathNode<'a>>;
// type MetadataVec<'a> = Vec<(PathNode<'a>, Option<Metadata>)>;
// type CanonicalVec<'a> = Vec<(PathNode<'a>, Option<CanonicalPath>)>;
// type PermVec<'a> = Vec<(PathNode<'a>, Option<(bool, bool, bool)>)>;

// struct CwdNode<'a>(Component<'a>); // If the path is not absolute, these are prepended to input nodes.
// struct InputNode<'a>(Component<'a>); // part of the input string

struct InputPath(PathBuf);

// struct Lol {
//     cwd: Option<PathBuf>, None when InputPath is absolute
//     input: InputPath,
//     metadata: Vec<Metadata>,
//     canonical: Vec<CanonicalPath>,
//     readlink: Vec<PathBuf>,
//     perms: Vec<(bool, bool, bool)>
// }

// Does not exist `./a/lol/foo.txt`
// - `./a/lol/foo.txt`
//      ^ File not a dir [✅ read, ✅ write, ❌ execute]
//          ↪ Absolute `/Users/rschneeman/Documents/a`
//          ↪ Canonical `/Users/rschneeman/Documents/a`

// Does not exist `./hello/lol/foo.txt`
// - `./hello/lol/foo.txt`
//      ^^^^^ - File, not a dir [✅ read, ✅ write, ❌ execute]
//            - Absolute `/Users/rschneeman/Documents/hello`
//            - Canonical `/Users/rschneeman/Documents/hello`

// From path does not exist `./hello/lol/foo.txt`
// - `./hello/lol/foo.txt`
//      ^^^^^ - File, not a dir [✅ read, ✅ write, ❌ execute]
//            - Absolute `/Users/rschneeman/Documents/hello`
//            - Canonical `/Users/rschneeman/Documents/hello`
// To path does not exist `./hello/lol/bar.txt`
// - `./hello/lol/bar.txt`
//      ^^^^^ - File, not a dir [✅ read, ✅ write, ❌ execute]
//            - Absolute `/Users/rschneeman/Documents/hello`
//            - Canonical `/Users/rschneeman/Documents/hello`

// From path does not exist `./hello/lol/foo.txt`
// To path does not exist `./hello/lol/bar.txt`
// - `./hello/lol`
//            ^^^ - Cannot stat (No such file or directory)
// - `./hello`
//      ^^^^^ - Dir [✅ read, ✅ write, ✅ execute]
//            - Absolute `/Users/rschneeman/Documents/hello`
//            - Canonical `/Users/rschneeman/Documents/hello`
// - `./hello`
//      ^^^^^
//      ├── `lolz`
//      ├── `there`
//      └── `rofl`

// exists `/path/to/directory/a/b/..`
//  - `/path/to/directory/a/b/..`
//                            ^^ - Dir [✅ read, ✅ write, ✅ execute]
//                               - Canonical `/path/to/directory/a`
//  - `/path/to/directory/`
//              ^^^^^^^^^
//              ├── `a` (exists)
//              └── `other.txt`

// does not exist `/path/to/directory/a/b/..`
//  - `/path/to/directory/a/b/..`
//                            ^^ - Cannot stat (No such file or directory)
//                               - Canonical `/path/to/directory/a`
// - `/path/to/directory`
//             ^^^^^^^^^ - File, not dir [✅ read, ✅ write, ✅ execute]
//  - `/path/to/`
//           ^^
//           ├── `lolz`
//           ├── `there`
//           ├── `directory` (exists)
//           └── `rofl`

// does not exist `a/b/..`
//  - `a/b/..`
//         ^^ - Cannot stat (No such file or directory)
//            - Absolute  `/path/to/directory/a/b/..`
// - `/path/to/directory`
//             ^^^^^^^^^ - File, not dir [✅ read, ✅ write, ✅ execute]
//  - `/path/to/`
//           ^^
//           ├── `lolz`
//           ├── `there`
//           ├── `directory` (exists)
//           └── `rofl`

// does not exist `../a/b/..`
//  - `../a/b/..`
//            ^^ - Cannot stat (No such file or directory)
//               - Absolute  `/path/to/directory/a/b/..`
//  - `..`
//     ^^ - File, not dir [✅ read, ✅ write, ✅ execute]
//        - Absolute  `/path/to/directory`
//        - Canonical `/path/to/directory`
// - `/path/to/directory`
//             ^^^^^^^^^ - File, not dir [✅ read, ✅ write, ✅ execute]
//  - `/path/to/`
//           ^^
//           ├── `lolz`
//           ├── `there`
//           ├── `directory` (exists)
//           └── `rofl`

// does not exist `../a/b/..`
//  - `../a/b/..`
//            ^^ - Cannot stat (No such file or directory)
//               - Absolute  `/path/to/directory/a/b/..`
//  - `../a/b`
//          ^ - File, not dir [✅ read, ✅ write, ✅ execute]
//            - Absolute  `/path/to/directory/a/b`
//            - Canonical `/path/to/directory/a/b`
// - `/path/to/directory/a`
//                       ^ - Dir [✅ read, ✅ write, ✅ execute]
//  - `/path/to/directory/a`
//                        ^
//                        ├── `lolz`
//                        ├── `there`
//                        ├── `b` (exists)
//                        └── `rofl`

// Is this a problem situation?

//  - `../a/b/..`
//            ^^ - Dir [✅ read, ✅ write, ✅ execute]
//               - Absolute  `/path/to/directory/../a/b/..`
//               - Canonical `/path/to/a/`
//  - `../a`
//        ^ - Dir [✅ read, ✅ write, ✅ execute]
//          - Absolute  `/path/to/directory/../a`
//          - Canonical `/path/to/a/`
//  - `../a`
//        ^
//        ├── `lolz`
//        ├── `b` (exists)
//        └── `rofl`

// Problem: situation:

//  - `../a/b/..`
//            ^^ - Dir [✅ read, ✅ write, ✅ execute]
//               - Absolute  `/path/to/directory/../a/b/..`
//               - Canonical `/path/to/a/`
//  - `../a`
//        ^ - Dir [✅ read, ✅ write, ✅ execute]
//          - Absolute  `/path/to/directory/../a`
//          - Canonical `/path/to/a/`
//  - `../a`
//        ^
//        ├── `lolz`
//        ├── `b` (exists)
//        └── `rofl`

// - Every bullet line shows exact path, never one in the middle

// - Every input gets a "this exact path" set of facts (lstat type)
// - On lstat fail - Some inputs gets a "this subset" set of facts for the last stat-able file/dir
//   - Okay to say "File, not dir" on an earlier path, it's true...but maybe not relevant.
// - Every input gets a dir with contents set of facts
//   - Last listable dir in input |
//   - Last listable dir in CWD (if relative)

// sketch:

struct DirInChain {
    path: std::path::PathBuf,
    absolute: AbsPath,
    canonical: CanonicalPath,
    perms: (bool, bool, bool),
    metadata: std::fs::Metadata,
    show: Vec<FileName>,
    entries: Vec<FileName>,
}

struct FileName(std::ffi::OsString);

struct Evidence {
    path: std::path::PathBuf,
    absolute: AbsPath,
    canonical: Option<CanonicalPath>,
    perms: Option<(bool, bool, bool)>,
    /// Required
    raw_metadata: Result<std::fs::Metadata, String>, // symlink_metadata()
    resolved_metadata: Option<Result<std::fs::Metadata, String>>, // metadata()
    readlink: Option<std::path::PathBuf>,
}

struct CaseFile {
    input: Evidence,
    last_stat: Option<Evidence>, // None if input is complete
    last_dir: Evidence,
    last_dir_contents: DirInChain,
}

// - Make a MetadataRaw and MetadataResolved type
// - Make a clone-able version of std::io::Error
//   - Preserve err kind, use for tri-state exists/doesn't/unknown
// - Make an interface for Perms that requires a MetadataResolved (since it follows symlinks)

// - Ok if we show semi-duplicate data as long as it's true (de-duping is HARD)
// - Avoid misleading information when possible

// exists `/tmp/lnk/dangling`
//  - `/tmp/lnk/dangling`
//              ^^^^^^^^ - Symlink → `../lnk/real/nope.txt`
//                       - Absolute → `/tmp/lnk/../lnk/real/nope.txt`
//                       - Cannot resolve target (No such file or directory)
//  - `/tmp/lnk/../lnk/real/nope.txt`
//                          ^^^^^^^^ - Cannot stat (No such file or directory)
//                                   - Absolute `/tmp/lnk/../lnk/real/nope.txt`
//                                   - Expanded `/tmp/lnk/real/nope.txt`
//  - `/tmp/lnk/../lnk/real`
//                     ^^^^ - Dir [✅ read, ✅ write, ✅ execute]
//                          - Canonical `/private/tmp/lnk/real`
//  - `/tmp/lnk/../lnk/real`
//                     ^^^^
//                     ├── `a`
//                     ├── `b`
//                     └── `c`

// Need to detect or handle symlink cycling

// exists `lnk/dangling`
//  - `lnk/cycling`
//         ^^^^^^^^ - Symlink → `../lnk/real/nope.txt`
//                  - Cannot resolve target (Too many levels of symbolic links)
//                  - Absolute `/tmp/lnk/cycling` → `/tmp/lnk/../lnk/real/nope.txt`
//  - `/tmp/lnk/../lnk/real/nope.txt`
//                          ^^^^^^^^ - Symlink → `/tmp/lnk/dangling`
//                                   - Cannot resolve target (Too many levels of symbolic links)

// - Show last symlink (if there is one)...maybe
//   - Skip common ones like `/tmp`
// - Not sure when we show a symlink or not

// - Maybe we use Means-Ends Analysis (MEA) and Goal Clobbering in automated planning?

// exists `/tmp/lnk/link_out/..`
//  - `/tmp/lnk/link_out/..`
//                       ^^ - Dir [✅ read, ✅ write, ✅ execute]
//                          - Canonical `/private/tmp/lnk/sub`
//  - `/private/tmp/lnk`
//              ^^^ - Dir [✅ read, ✅ write, ✅ execute]
//                  - Canonical `/private/tmp/lnk/sub`
//  - `/tmp/lnk`
//          ^^^
//          ├── `lorem`
//          ├── `sub` (exists)
//          ├── `ipsem`
//          └── `other`

// exists `/tmp/lnk/link_out/..`
//  - `/tmp/lnk/link_out/..`
//              ^^^^^^^^ → `/tmp/lnk/sub`
//  - `/tmp/lnk/sub/..`
//                  ^^ → `/tmp/lnk/sub`

//
//  - `/tmp/lnk/link_out/..`
//              ^^^^^^^^ → `/tmp/lnk/sub`
//
//                       ^^ - Dir [✅ read, ✅ write, ✅ execute]
//                          - Canonical `/private/tmp/lnk/sub`
//  - `/tmp/lnk/link_out`
//              ^^^^^^^^ - Symlink → `/tmp/lnk/sub`
//                       - Canonical `/private/tmp/lnk/sub`
//  - `/tmp/lnk/sub`
//              ^^^ - Dir [✅ read, ✅ write, ✅ execute]
//                  - Canonical `/private/tmp/lnk/sub`
//  - `/tmp/lnk`
//          ^^^
//          ├── `lorem`
//          ├── `sub` (exists)
//          ├── `ipsem`
//          └── `other`

// From path does not exist `./hello/lol/foo.txt`
// To path does not exist `./hello/lol/bar.txt`
// - `./hello`
//      ^^^^^ - Cannot stat (No such file or directory)
//            - Absolute `/Users/rschneeman/Documents/hello`
// - `/Users/rschneeman/Documents`
//                      ^^^^^^^^^ - Dir [✅ read, ✅ write, ✅ execute]
//                                - Canonical `/Users/rschneeman/Documents`
// - `/Users/rschneeman/Documents`
//                      ^^^^^^^^
//                      ├── `lolz`
//                      ├── `there`
//                      └── `rofl`

// From path does not exist `./hello/lol/foo.txt`
// - `./hello/lol/foo.txt`
//                ^^^^^^^ - Cannot stat (No such file or directory)
//                        - Absolute  `/Users/rschneeman/Documents/hello/lol/foo.txt`
// To path exists `./hello/lol/bar.txt`
// - `./hello/lol/bar.txt`
//                ^^^^^^^ - File [✅ read, ✅ write, ✅ execute]
//                        - Absolute  `/Users/rschneeman/Documents/hello/lol/bar.txt`
//                        - Canonical `/Users/rschneeman/Documents/hello/lol/bar.txt`
// - `./hello/lol`
//            ^^^ - Dir [✅ read, ✅ write, ✅ execute]
//                - Absolute  `/Users/rschneeman/Documents/hello/lol`
//                - Canonical `/Users/rschneeman/Documents/hello/lol`
//                - Shared by to and from paths
// - `./hello/lol`
//            ^^^
//            ├── `lolz`
//            ├── `there`
//            ├── `bar.txt` (exists)
//            └── `rofl`
//                 ❌ <Missing `foo.txt`>

// - `/path/to/no_exec/file.txt`
//                     ^^^^^^^^ - Cannot stat (Permission denied)
// - `/path/to/no_exec`
//             ^^^^^^^ - Dir [✅ read, ✅ write, ❌ execute]
//                     - Files in a directory missing execute can be listed but not traversed
// - `/path/to/no_exec`
//             ^^^^^^^
//             └── `file.txt` (exists)

// From path exists `./hello/lol/foo.txt`
// - `./hello/lol/foo.txt`
//                ^^^^^^^ - File [✅ read, ✅ write, ✅ execute]
//                        - Absolute  `/Users/rschneeman/Documents/hello/lol/foo.txt`
//                        - Canonical `/Users/rschneeman/Documents/hello/lol/foo.txt`
// To path exists `./hello/lol/bar.txt`
// - `./hello/lol/bar.txt`
//                ^^^^^^^ - File [✅ read, ✅ write, ✅ execute]
//                        - Absolute  `/Users/rschneeman/Documents/hello/lol/bar.txt`
//                        - Canonical `/Users/rschneeman/Documents/hello/lol/bar.txt`
// - `./hello/lol`
//            ^^^ - Dir [✅ read, ✅ write, ✅ execute]
//                - Absolute `/Users/rschneeman/Documents/hello/lol`
//                - Canonical `/Users/rschneeman/Documents/hello/lol`
//                - Shared by to and from paths
// - `./hello/lol`
//            ^^^
//            ├── `lolz`
//            ├── `there`
//            ├── `bar.txt` (exists)
//            ├── `foo.txt` (exists)
//            └── `rofl`

// - `./hello/lol`
//            ^^^ - Cannot stat (No such file or directory)
// - `./hello`
//      ^^^^^ - Dir [✅ read, ✅ write, ✅ execute]
//            - Absolute `/Users/rschneeman/Documents/hello`
//            - Canonical `/Users/rschneeman/Documents/hello`
// - `./hello`
//      ^^^^^
//      ├── `lolz`
//      ├── `there`
//      └── `rofl`

// From path does not exist `./hello/lol/foo.txt`
// To path does not exist `./hello/lol/bar.txt`
// - `./hello/`
//      ^^^^^ - File, not a dir [✅ read, ✅ write, ❌ execute]
//            - Absolute `/Users/rschneeman/Documents/hello`
//            - Canonical `/Users/rschneeman/Documents/hello`

// Does not exist `./hello/lol/foo.txt`
// - `./hello/lol/foo.txt`
//      ^^^^^ File, not a dir [✅ read, ✅ write, ❌ execute]
// - `./hello/lol/foo.txt`
//      ^^^^^ Canonical `/Users/rschneeman/Documents/a`
//
//
//
//      ↪ `/Users/rschneeman/Documents/a`
//      ^ File, not a dir
//      ↪ `/Users/rschneeman/Documents/a`
//      ^ File, not a dir
//        [✅ read, ✅ write, ❌ execute]
// - `./a` → `/Users/rschneeman/Documents/a`
//  lol/foo.txt`
//                                ^ File, not a dir
//                                  [✅ read, ✅ write, ❌ execute]
// - `/Users/rschneeman/Documents/a`

// Dir/File/

// Can always show additive data CWD + original
//

// struct

// struct SymlinkRaw(Component, Metadata, Perms);
// struct SymlinkResolved(Component, FileOrDir);

// enum FileOrDir {
//     File(FileResolved),
//     Dir(DirResolved),
// }

// struct DirResolved(Component, Metadata);
// struct FileResolved(Component, Metadata);

/// A [`HappyPath`] represents a path with no problems that we could find
///
/// For a path to be happy, it's parent (directory) must be good too, represented by a [`DirOk`]
#[derive(Debug)]
pub(crate) struct KnownPath {
    pub(crate) absolute: AbsPath,
    pub(crate) canonical: CanonicalPath,
    pub(crate) symlink_target: Option<AbsPath>,
    pub(crate) resolved_type: ResolvedType,
    pub(crate) parent: DirOk,
    pub(crate) read: bool,
    pub(crate) write: bool,
    pub(crate) execute: bool,
}

mod stat_path {
    use crate::{
        abs_path::{self, AbsPath, AbsPathError, RelativePath},
        canonical_path::CanonicalPath,
    };
    use std::path::{Path, PathBuf};
    use std::{fs::Metadata, path::Component};

    // Need something like ExpandPath but start from root like here
    // Once I have that then I can do what?
    // Stress test it with an LLM I think

    // Make a struct that translatest to -> The exact Stat of the thing you gave me | Reason why not
    // where reason why not can return a Stat of a prior dir.

    /// Path with lstat `symlink_metadata` attached
    ///
    /// - Path exists
    /// - May be a symlink, may or may not resolve
    #[derive(Debug)]
    struct RawStatPath {
        /// Original
        raw: PathBuf,
        base: CanonicalPath,
        /// To canonicalize a path it must  if metadata succeeds, but canonical fails on the input
        rest: Option<RelativePath>,
        raw_metadata: Metadata,
    }

    #[derive(Debug)]
    enum RawStatError {
        AbsError(AbsPathError),
        Prior(RawStatPath),
        CannotCanonicalizeAnything {
            root: PathBuf,
            error: std::io::Error,
        },
        CanonicalOkMetadataFailed {
            error: std::io::Error,
            canonical: CanonicalPath,
        },
    }

    fn split_root(path: impl AsRef<Path>) -> (PathBuf, PathBuf) {
        let mut parts = path.as_ref().components().into_iter();
        let mut base = PathBuf::new();
        while let Some(part) = parts.next() {
            match part {
                Component::Prefix(_) => {
                    base.push(part);
                }
                Component::RootDir => {
                    base.push(part);
                    break;
                }
                _ => unreachable!("Root dir must come before any other path Component"),
            }
        }
        (base, parts.collect())
    }

    impl RawStatPath {
        fn once(raw: impl AsRef<Path>, absolute: &AbsPath) -> Result<Self, std::io::Error> {
            let path = raw.as_ref();
            let metadata = path.symlink_metadata()?;
            let canonical = CanonicalPath::new(&absolute)?;

            Ok(RawStatPath {
                raw: path.to_path_buf(),
                base: canonical,
                rest: None,
                raw_metadata: metadata,
            })
        }

        fn new(path: impl AsRef<Path>) -> Result<RawStatPath, RawStatError> {
            let path = path.as_ref();
            let metadata = path.symlink_metadata();
            let absolute = AbsPath::new(path).map_err(RawStatError::AbsError)?;

            if let Ok(result) = RawStatPath::once(path, &absolute) {
                return Ok(result);
            }

            let (mut root, rest) = split_root(&absolute);
            let mut last = RawStatPath::once(&root, &AbsPath::new(&root).expect("is absolute"))
                .map_err(|error| RawStatError::CannotCanonicalizeAnything {
                    root: root.clone(),
                    error,
                })?;

            for part in rest.components() {
                root.push(part);
                match RawStatPath::once(&root, &AbsPath::new(&root).expect("is absolute")) {
                    Ok(raw) => {
                        last = raw;
                    }
                    Err(_) => break,
                }
            }
            Ok(Self::test_next_rest(last, absolute).unwrap())
        }

        fn path(&self) -> PathBuf {
            let mut path = self.base.as_ref().to_path_buf();
            if let Some(rest) = self.rest.clone() {
                for part in rest.as_ref().components() {
                    path.push(part);
                }
                path
            } else {
                path
            }
        }

        // Write tests
        // do a loop of CanonicalPath, then test the next rest, and then backoff.
        fn test_next_rest(last: RawStatPath, original_absolute: AbsPath) -> Result<Self, ()> {
            // calculate the leftover, if it can Metadata, then #shipit

            // Full path
            let mut leftovers = original_absolute.as_ref().components();

            // built from same components, remove same components
            for part in last.raw.components() {
                leftovers.next();
            }

            let mut next_path = last.base.as_ref().to_path_buf();
            if let Some(rest) = leftovers.next() {
                next_path.push(rest);

                if let Ok(metadata) = std::fs::symlink_metadata(&next_path) {
                    Ok(Self {
                        raw: next_path,
                        base: last.base,
                        rest: Some(RelativePath::new(rest).expect("relative path")),
                        raw_metadata: metadata,
                    })
                } else {
                    Err(())
                }
            } else {
                Err(())
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_valid_path() {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().canonicalize().unwrap();

            let path = dir.join("hello.txt");
            std::fs::write(&path, "world").unwrap();
            let out = RawStatPath::new(&path).unwrap();

            let RawStatPath {
                raw,
                base,
                rest,
                raw_metadata,
            } = out;

            assert_eq!(raw, path);
            assert_eq!(base.as_ref(), &std::fs::canonicalize(path).unwrap());
            assert_eq!(rest, None);
            assert!(raw_metadata.is_file());
        }

        #[test]
        #[cfg(unix)]
        fn path_exists_not_resolvable() {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().canonicalize().unwrap();

            let path = dir.join("exists.txt");
            std::os::unix::fs::symlink(dir.join("does-not-exist.txt"), &path).unwrap();
            let out = RawStatPath::new(&path).unwrap();

            let RawStatPath {
                raw,
                base,
                rest,
                raw_metadata,
            } = out;

            assert_eq!(raw, path);
            assert_eq!(base.as_ref(), &std::fs::canonicalize(dir).unwrap());
            assert_eq!(rest, Some(RelativePath::new("exists.txt").unwrap()));
            assert!(raw_metadata.is_symlink());
        }
    }
}

/// File exists but may or may not be resolvable, path is fully normalized
///
/// If it was just `ExistPath(PathBuf) it would provide a s slightly weaker guarantee than a
/// CanonicalPath which is guaranteed to be resolvable. This can occur when:
///
/// - File is a symlink with a problem
/// - File is in a directory and the directory is missing the `execute` permission
///   but has the `read` permission. This would allow the directory contents to
///   be listed (existance seen) but not "traversed" (referenced to read the actual
///   file or a file after it [when a dir]).
///
/// These cases are enumerated and (lots) more data pre-computed
#[derive(Debug)]
pub(crate) enum ExistPath {
    /// Everything in place, no guesses
    Ok {
        path: CanonicalPath,
        readlink: Option<PathBuf>,
        resolved_metadata: Metadata,
    },
    /// File technically exists but it's a symlink that cannot be resolved
    BadSymlink {
        path: ExpandPath,
        raw_metadata: Metadata,
        readlink: Result<PathBuf, std::io::Error>,
    },
    /// File cannot be accessed directly, but parent exists and is listable
    ParentMissingExec {
        path: ExpandPath,
        parent: CanonicalPath,
        raw_metadata: Metadata,
        siblings: Vec<CanonicalPath>,
    },
}

enum ExistPathProblem {
    DoesNotExist(ExpandPath),
    CannotCanonicalizeAnything(CannotCanonicalizeAnything),
    ErrorTOCTOU {
        error: std::io::Error,
        explanation: String,
    },
}

impl ExistPath {
    pub(crate) fn new(path: AbsPath) -> Result<Self, ExistPathProblem> {
        let path = ExpandPath::new(&path).map_err(ExistPathProblem::CannotCanonicalizeAnything)?;
        let raw_metadata = std::fs::symlink_metadata(path.as_ref());
        match path {
            ExpandPath::Canonical(path) => {
                let mut readlink = None;
                let resolved_metadata = std::fs::metadata(path.as_ref()).map_err(|error| {
                    ExistPathProblem::ErrorTOCTOU {
                        error,
                        explanation: String::from(
                            "Canonical path found but metadata unreadable `std::fs::metadata`",
                        ),
                    }
                })?;
                if raw_metadata
                    .map_err(|error| ExistPathProblem::ErrorTOCTOU { error, explanation: String::from("Canonical path found but raw metadata unreadable `std::fs::symlink_metadata`") })?
                    .is_symlink() {
                        readlink = Some(std::fs::read_link(path.as_ref())
                        .map_err(|error| ExistPathProblem::ErrorTOCTOU { error, explanation: String::from("canonical path found and raw metadata readable, is a symlink but readlink failed `std:;fs::read_link`") })?);
                }

                Ok(Self::Ok {
                    path,
                    readlink,
                    resolved_metadata,
                })
            }
            ExpandPath::Partial(_) => match raw_metadata {
                Ok(raw_metadata) => {
                    // file exists but couldn't be canonicalized, by induction it's a broken symlink
                    if raw_metadata.is_symlink() {
                        let readlink = std::fs::read_link(path.as_ref());
                        Ok(ExistPath::BadSymlink {
                            path,
                            raw_metadata,
                            readlink,
                        })
                    } else {
                        // Unexpected
                        Err(ExistPathProblem::ErrorTOCTOU {
                            error: std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "not a symlink",
                            ),
                            explanation: String::from(
                                "File exists `std::fs::symlink_metadata` but could not be canonicalized. Must be a symlink but `is_symlink()` false",
                            ),
                        })
                    }
                }
                Err(error) => {
                    match (
                        AbsPath::from(path.clone())
                            .parent()
                            .map(|parent| CanonicalPath::new(&parent)),
                        path.as_ref().file_name(),
                    ) {
                        (Some(Ok(parent)), Some(filename)) => match parent.read_dir() {
                            Ok(entries) => todo!(), // check if the path is in an entry
                            Err(_) => Err(ExistPathProblem::DoesNotExist(path)),
                        },
                        _ => Err(ExistPathProblem::DoesNotExist(path)),
                    }
                }
            },
        }
    }
}

#[derive(Debug, Clone)]
struct HappyDir {
    canonical: CanonicalPath,
    entries: Vec<ExpandPath>,
    read: bool,
    write: bool,
    execute: bool,
}

// enum PriorPathProblem {
//     File {
//         canonical: CanonicalPath,
//         parent: DirExist,
//     },
//     BrokenLink {
//         path: ExpandPath,
//         raw_metadata: Metadata,
//         readlink: PathBuf,
//         parent: DirExist,
//     },
//     ErrorTOCTOU {
//         error: std::io::Error,
//         reason: String,
//     },
// }

// #[derive(Debug)]
// enum ExistPathProblem {
//     ///
//     NoParentIsRoot(PathBuf),

//     /// An edge case happened that shouldn't Either representes TOCTOU or internal logic error
//     ErrorTOCTOU {
//         error: std::io::Error,
//         explanation: String,
//     },
// }

// impl DirExist {
//     fn new(canonical: CanonicalPath) -> Result<Self, std::io::Error> {
//         let read = canonical.as_ref().access(AccessMode::READ).is_ok();
//         let write = canonical.as_ref().access(AccessMode::WRITE).is_ok();
//         let execute = canonical.as_ref().access(AccessMode::EXECUTE).is_ok();

//         let mut entries = Vec::new();
//         if read {
//             entries = canonical.read_dir()?;
//         } else {
//             entries = Vec::new();
//         }

//         Ok(Self {
//             canonical,
//             entries,
//             read,
//             write,
//             execute,
//         })
//     }
// }

/// A [`DirOk`] represents a directory with no problems that we could find
#[derive(Debug, Clone)]
pub(crate) struct DirOk {
    #[allow(dead_code)]
    pub(crate) absolute: AbsPath,
    pub(crate) canonical: CanonicalPath,
    pub(crate) entries: Vec<AbsPath>,
    pub(crate) read: bool,
    pub(crate) write: bool,
    pub(crate) execute: bool,
}

impl DirOk {
    pub(crate) fn new(absolute: AbsPath) -> Result<Self, std::io::Error> {
        let canonical = CanonicalPath::new(&absolute)?;
        let entries = absolute.read_dir()?;

        let read = true;
        let write = canonical.as_ref().access(AccessMode::WRITE).is_ok();
        let execute = canonical.as_ref().access(AccessMode::EXECUTE).is_ok();

        Ok(DirOk {
            absolute,
            canonical,
            entries,
            read,
            write,
            execute,
        })
    }

    pub(crate) fn has_entry(&self, path: &AbsPath) -> bool {
        self.entries.contains(path)
    }
}

#[derive(Debug)]
pub(crate) enum UnknownPath {
    AbsPathError(abs_path::AbsPathError),
    IsRoot(AbsPath),
    CannotCanonicalizeAnything(CannotCanonicalizeAnything),
    ParentProblem {
        #[allow(dead_code)] // Prove we can access CWD and path is not empty
        absolute: AbsPath,
        expand: ExpandPath,
        parent: AbsPath,
        /// Original error preventing us from creating a `DirOk` for the parent directory.
        /// Not printed, we traverse prior directories to find the root cause
        _error: std::io::Error,
    },
    DoesNotExist {
        absolute: AbsPath,
        expand: ExpandPath,
        parent: DirOk,
    },
    // Path exists, but we cannot canonicalize it
    CannotCanonicalize {
        absolute: AbsPath,
        expand: ExpandPath,
        parent: DirOk,
        error: std::io::Error,
    },
    /// Path exists, but we cannot read the metadata
    /// TOCTOU likely: Path exists and can be canonicalized, but we cannot read the metadata
    ///
    /// Usually this would cause a CannotCanonicalize error, but if there is a TOCTOU race condition
    /// where the parent directory has read and execute access when the canonicalization is attempted,
    /// but loses execute access before the metadata reading, then this error will occur.
    CannotMetadata {
        absolute: AbsPath,
        canonical: CanonicalPath,
        parent: DirOk,
        error: std::io::Error,
    },
    /// Path exists, but and is reportedly a symlink but readlink fails
    /// Probably TOCTOU otherwise the canonical path would have errored
    CannotReadLink {
        absolute: AbsPath,
        canonical: CanonicalPath,
        parent: DirOk,
        error: std::io::Error,
    },
}

pub(crate) fn state(path: &Path) -> Result<KnownPath, Box<UnknownPath>> {
    let absolute = AbsPath::new(path).map_err(UnknownPath::AbsPathError)?;
    let abs_parent = absolute
        .parent()
        .ok_or_else(|| UnknownPath::IsRoot(absolute.clone()))?;
    let expand = ExpandPath::new(&absolute).map_err(UnknownPath::CannotCanonicalizeAnything)?;
    let parent = DirOk::new(abs_parent.clone()).map_err(|error| UnknownPath::ParentProblem {
        absolute: absolute.clone(),
        expand: expand.clone(),
        parent: abs_parent.clone(),
        _error: error,
    })?;
    let path_does_not_exist = !parent.has_entry(&absolute);
    let canonical = CanonicalPath::new(&absolute).map_err(|error| {
        if path_does_not_exist {
            UnknownPath::DoesNotExist {
                absolute: absolute.clone(),
                expand: expand.clone(),
                parent: parent.clone(),
            }
        } else {
            UnknownPath::CannotCanonicalize {
                absolute: absolute.clone(),
                expand: expand.clone(),
                parent: parent.clone(),
                error,
            }
        }
    })?;

    let resolved_type = ResolvedMetadata::new(&absolute)
        .map_err(|error| UnknownPath::CannotMetadata {
            absolute: absolute.clone(),
            canonical: canonical.clone(),
            parent: parent.clone(),
            error,
        })?
        .resolved_type();
    let symlink_target =
        abs_path::try_readlink(&absolute).map_err(|error| UnknownPath::CannotReadLink {
            absolute: absolute.clone(),
            canonical: canonical.clone(),
            parent: parent.clone(),
            error,
        })?;

    let read = canonical.as_ref().access(AccessMode::READ).is_ok();
    let write = canonical.as_ref().access(AccessMode::WRITE).is_ok();
    let execute = canonical.as_ref().access(AccessMode::EXECUTE).is_ok();

    Ok(KnownPath {
        absolute,
        canonical,
        symlink_target,
        resolved_type,
        parent,
        read,
        write,
        execute,
    })
}

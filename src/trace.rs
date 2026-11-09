//! Trace a lexical path to physical nodes
//!
//! A [`Trace`] walks a [`Path`] one [`Component`] at a time, starting from the root. Each [`Step`]
//! records what the filesystem reported for its component as a [`PhysicalNode`], or the best-effort
//! reason it couldn't learn more.
//!
//! The walk stops at the first component it can't see past.

use crate::abs_path::{readlink, AbsPath, AbsPathError};
use crate::canonical_path::{CannotCanonicalizeAnything, CanonicalPath, Entry};
use crate::component::{self, NormalComponent, OwnedComponent, ParentDirComponent};
use crate::non_empty::NonEmpty;
use faccess::{AccessMode, PathExt};
use std::borrow::Cow;
use std::convert::TryFrom;
use std::io::Write;
use std::io::{stderr, ErrorKind};
use std::path::{Component, Path, PathBuf};

/// Every [`Component`] of a path, and the physical contents at each step
///
/// Constructed from the root and walked to the end of the input [`Path`]
#[derive(Debug)]
pub(crate) struct Trace {
    /// The original, unmodified, input [`Path`]
    input: PathBuf,

    /// The input path, but absolute. All walks start from the root.
    /// When the input path was already absolute this will match.
    absolute: AbsPath,

    /// The physical, resolved root of the input path
    root: CanonicalPath,

    /// Details about the physical contents of each [`Component`] that comes after the
    /// root in the path.
    steps: NonEmpty<Step>,
}

/// What a single, physical disk check reported for one [`Component`]
#[derive(Debug)]
pub(crate) enum PhysicalNode {
    /// A directory
    Directory(CanonicalPath),

    /// Not a directory and not a symlink. It cannot hold another entry
    File(CanonicalPath),

    /// A symlink
    ///
    /// - `target` is where the link points without following it (lexical).
    ///   An error is returned when there's a permissions mismatch between what `symlink_metadata` needs,
    ///   which tells us a file is a symlink, and what `read_link` needs, which tells us what the
    ///   symlink holds. Or on TOCTOU.
    /// - `resolved` is where the link resolves to (physical).
    ///   This can fail for a number of reasons including when a symlink produces an infinite/circular
    ///   chain. Be careful trying to traverse the target of an unresolvable result.
    Symlink {
        target: Result<(PathBuf, AbsPath), std::io::Error>,
        resolved: Result<ResolvedSymlink, std::io::Error>,
    },

    /// Entry is a lexical `..` (parent dir)
    ///
    /// Safe to apply lexically: a [`CanonicalPath`] holds no symlinks, and POSIX defines
    /// `..` on a symlink free path as removing the last component. Root is its own parent,
    /// matching `realpath` on `/..`.
    ParentDir {
        /// The directory folded e.g. for an input of `/a/b/c/..` the directory `/a/b/c` holds `..`
        /// and folded below would be `/a/b/c`
        #[allow(dead_code)]
        folded: CanonicalPath,
        /// Where `..` landed e.g. `/a/b/c/..` → `/a/b`
        resolved: CanonicalPath,
    },

    /// Can list entries of the parent directory, but cannot access/traverse individual items
    ParentNoExec {
        /// The directory holding the name
        parent: CanonicalPath,
        /// Some if the directory has the entry, otherwise None
        entry: Option<NormalComponent>,
        /// This error from `symlink_metadata` on the full path triggered the permissions lookup.
        #[allow(dead_code)]
        error: std::io::Error,
    },

    /// Nothing is at this name
    ///
    /// This is constructed when:
    ///
    /// - The walk reached the directory holding it
    /// - [`std::fs::symlink_metadata`] on the name fails with [`ErrorKind::NotFound`]
    /// - It is not a [`PhysicalNode::ParentNoExec`]
    /// - It is not a [`PhysicalNode::Raced`]
    Missing(std::io::Error),

    /// When no other classification fits
    ///
    /// The directory it sits in exists, but [`CanonicalPath::entry`] failed in a way
    /// that fits no better classification.
    ///
    /// Some non-exhaustive examples:
    ///
    /// - A directory denying both search and listing
    /// - A component longer than the filesystem accepts
    /// - Failing hardware, or a stale handle on a network mount
    /// - A directory changed in a way the race check could not see
    UnknownLookup(std::io::Error),

    /// Two filesystem calls contradicted each other
    ///
    /// For example (hypothetically) if one metadata call said a path is a directory, and another
    /// said it is a file (not a symlink) then that would strongly indicate there's a TOCTOU issue
    /// or some other race condition happening under the hood. This variant is used when we can
    /// strongly assert two facts disagree.
    ///
    /// When the variant is something other than `Raced` it means that no race condition was caught,
    /// not that nothing moved.
    Raced {
        /// The reason we believe a race condition exists
        why: &'static str,
        /// The error that triggered the investigation
        error: std::io::Error,
    },

    /// Never examined, because an earlier component stopped the walk
    NotReached,
}

/// Where following a symlink landed
#[derive(Debug)]
pub(crate) enum ResolvedSymlink {
    /// The link lands on a directory. The walk can continue through it.
    Dir(CanonicalPath),
    /// The link lands on a file which stops the walk.
    File(CanonicalPath),
}

impl ResolvedSymlink {
    /// Where the link landed, whatever is there
    pub(crate) fn path(&self) -> &CanonicalPath {
        match self {
            ResolvedSymlink::Dir(path) | ResolvedSymlink::File(path) => path,
        }
    }
}

impl PhysicalNode {
    /// The physical location this component resolved to
    ///
    /// `None` when something about the physical lookup failed.
    pub(crate) fn resolved_to(&self) -> Option<Cow<'_, CanonicalPath>> {
        match self {
            PhysicalNode::Directory(path) | PhysicalNode::File(path) => Some(Cow::Borrowed(path)),
            PhysicalNode::Symlink { resolved, .. } => resolved
                .as_ref()
                .ok()
                .map(|landed| Cow::Borrowed(landed.path())),
            PhysicalNode::ParentDir { resolved, .. } => Some(Cow::Borrowed(resolved)),
            PhysicalNode::Missing(_)
            | PhysicalNode::UnknownLookup(_)
            | PhysicalNode::Raced { .. }
            | PhysicalNode::NotReached => None,
            PhysicalNode::ParentNoExec { parent, entry, .. } => entry
                .as_ref()
                .map(|entry| unsafe { Cow::Owned(parent.unchecked_join(entry)) }),
        }
    }
}

/// Details about physical contents of one [`Component`] on a path
#[derive(Debug)]
pub(crate) struct Step {
    /// Indexes [`Trace::input`] by its [`Path::components`].
    ///
    /// For example, if the input is `/a/b/c` then the components will be
    /// `[RootDir, Normal("a"), Normal("b"), Normal("c")]`. The step
    /// index for `Normal("a")` would be `Some(1)`.
    ///
    /// Each step is built from the absolute path, which can differ from the input.
    /// Is `None` when the two differ at or after this step, which happens when:
    ///
    /// - The step comes from CWD and the path was relative.
    /// - On Windows, [`std::path::absolute`] rewrote the input. It folds `..`, so `a\b\..`
    ///   becomes `<cwd>\a`, and it strips a trailing `.`, so `a\b\c.` becomes `<cwd>\a\b\c`.
    pub(crate) input_index: Option<usize>,

    /// One component of the absolute path, a name or `..`. Never a root or `.`
    ///
    /// Can differ from the input, see [`Step::input_index`].
    pub(crate) name: OwnedComponent,

    /// Physical(ish) representation of the component
    ///
    /// When this field is `Some`, it provides a path who's lexical and physical representations do
    /// not diverge due to symlinks in prior directories. Meaning, every prior directory is either
    /// expanded/resolved or is proved to not be a symlink via proving it does not exist.
    /// e.g. if `/a` exists and `/a/b` provably does not, it is safe to say that the physical
    /// and lexical representation of `/a/b/c` do not diverge.
    ///
    /// If this component exists and is a symlink, it represents the physical path to that symlink (does not follow
    /// the symlink). For a regular directory or file, or a `..`, it is the same as a
    /// [`CanonicalPath`].
    ///
    /// It is `None` when we cannot provably construct a path that does not diverge. For example
    /// if we could not resolve a symlink of a prior directory, we cannot confidently construct
    /// a physical path.
    pub(crate) at: Option<AbsPath>,

    /// What was found when we looked on disk
    pub(crate) contents: PhysicalNode,
}

/// A directory and the next lexical [`Component`]
///
/// e.g. for `/a/b/c` if `b` is the directory, `c` would be the entry.
///
/// If the last entry is `..` then the entry is where it resolved to.
/// e.g. for `/x/y/z/..` the `..` lands on `/x/y`, so the directory is `/x` and the entry is `y`.
///
/// See [`Trace::listing`] for details.
#[derive(Debug)]
pub(crate) struct DirBeforeComponent {
    /// Directory
    pub(crate) dir_before: CanonicalPath,
    /// Next lexical (not physical) component on the path (may or may not be on disk).
    /// For a `..`, this is the name of the directory it lands on.
    pub(crate) component: NormalComponent,
}

/// Errors when we could not construct a [`Trace`]
///
/// Even though this is an "error" it's treated as data about the input path that we still
/// use to output fact(s) about the input path.
#[derive(Debug)]
pub(crate) enum CannotTrace {
    /// Error while constructing an [`AbsPath`] from the input
    CannotAbsolute(AbsPathError),

    /// The root of the input path could not be canonicalized
    RootNotReachable(CannotCanonicalizeAnything),

    /// A [`Trace`] holds a root and at least one [`Component`] as [`Step`]s. If
    /// the input is only a root, we cannot produce a valid [`Trace`]
    IsRoot(CanonicalPath),
}

impl Trace {
    pub(crate) fn new(input: &Path) -> Result<Trace, CannotTrace> {
        let absolute = AbsPath::new(input).map_err(CannotTrace::CannotAbsolute)?;

        let root = absolute.lex_root();
        let root = CanonicalPath::new(&root).map_err(|root_error| {
            CannotTrace::RootNotReachable(CannotCanonicalizeAnything {
                original: absolute.clone(),
                root,
                root_error,
            })
        })?;

        let mut position = WalkPosition::Dir(root.clone());
        let mut steps = Vec::new();

        for component in absolute.as_ref().components() {
            let (step, next) = match component::owned(component) {
                // `Prefix` + `RootDir` come from the starting canonical path.
                // `CurDir` references the last reached directory
                OwnedComponent::Prefix(_)
                | OwnedComponent::RootDir(_)
                | OwnedComponent::CurDir(_) => continue,
                OwnedComponent::Normal(normal_component) => enter(position, normal_component),
                OwnedComponent::ParentDir(parent_dir_component) => {
                    up(position, parent_dir_component)
                }
            };

            steps.push(step);
            position = next;
        }

        let mut steps = NonEmpty::try_from(steps).map_err(|_| CannotTrace::IsRoot(root.clone()))?;

        attribute(&mut steps, input);

        Ok(Trace {
            input: input.to_path_buf(),
            absolute,
            root,
            steps,
        })
    }

    /// [`CanonicalPath`] of the full input if all components could be visited
    pub(crate) fn physical_location(&self) -> Option<CanonicalPath> {
        if self.stopped_early_at().is_some() {
            None
        } else {
            self.steps
                .last()
                .contents
                .resolved_to()
                .map(Cow::into_owned)
        }
    }

    // The step of the last component of the input path
    #[cfg(test)]
    pub(crate) fn last_step(&self) -> &Step {
        self.steps.last()
    }

    /// The first step where the walk found a race condition
    #[cfg(test)]
    pub(crate) fn raced(&self) -> Option<&Step> {
        self.steps
            .iter()
            .find(|step| matches!(step.contents, PhysicalNode::Raced { .. }))
    }

    /// Appends a [`PhysicalNode::Raced`] variant for tests
    #[cfg(test)]
    pub(crate) fn append_race(&mut self, why: &'static str, error: std::io::Error) {
        let index = StepCursor::trace_stopped_at(self).index;
        self.steps[index].contents = PhysicalNode::Raced { why, error };
    }

    /// The [`Step`] that prevented the full path from being walked
    pub(crate) fn stopped_early_at(&self) -> Option<&Step> {
        let cursor = StepCursor::trace_stopped_at(self);
        let step = cursor.current();

        // There's another step after this one
        let stopped = cursor.after().is_some() ||
                // The walk reached the last component but couldn't resolve it
                step.contents.resolved_to().is_none();
        stopped.then_some(step)
    }

    /// Directory to show on the report
    ///
    /// Returns the last directory before the normal component where the walk stopped.
    /// For example, here the walk stopped at `the` and the last directory is `before`:
    ///
    /// ```text
    /// /the/dir/before/the/file.txt
    ///                 ^^^ component
    ///          ^^^^^^ dir_before
    /// ```
    ///
    /// If the component would have been a `..` it will return the entry it folded to. For example,
    /// if the full path exists:
    ///
    /// ```text
    /// /the/dir/before/eaten/..
    ///          ^^^^^^ component
    ///      ^^^ dir_before
    /// ```
    ///
    /// The `..` component points/folds to `before` and the directory that holds it is `dir`.
    ///
    /// If someone is to list the contents of the directory, the component can be used to check if
    /// there is a directory entry with that name.
    ///
    /// Returns `None` when it cannot return a dir and a component. This happens when the path resolves
    /// to root like `/a/../b/..`.
    pub(crate) fn listing(&self) -> Option<DirBeforeComponent> {
        let cursor = StepCursor::trace_stopped_at(self);
        let step = cursor.current();

        // Convert `..` to a normalized component that can be checked against contents of the directory.
        if let PhysicalNode::ParentDir { resolved, .. } = &step.contents {
            return Some(DirBeforeComponent {
                dir_before: resolved.parent()?,
                component: resolved.filename_component()?,
            });
        }

        // Extract directory from the step prior to the one that stopped
        let dir = match cursor.before() {
            None => self.root.clone(),
            Some(previous) => {
                let Some(physical) = previous.contents.resolved_to() else {
                    // Shouldn't happen, warn if it does
                    warn(format!(
                        "trace for `{}` stopped at `{}`. Prior component `{}` was not resolvable",
                        self.input.display(),
                        step.name.as_ref().to_string_lossy(),
                        previous.name.as_ref().to_string_lossy()
                    ));
                    return None;
                };
                physical.into_owned()
            }
        };

        Some(DirBeforeComponent {
            dir_before: dir,
            component: step.name.normal()?.clone(),
        })
    }

    /// Lexical parent of the last [`Step::at`]
    #[cfg(test)]
    pub(crate) fn last_step_expanded_lexical_parent(&self) -> Option<AbsPath> {
        self.steps.last().at.as_ref()?.lex_parent()
    }

    /// The unmodified original path
    #[cfg(test)]
    pub(crate) fn input(&self) -> &Path {
        &self.input
    }

    /// An absolute version of the original path
    pub(crate) fn absolute(&self) -> &AbsPath {
        &self.absolute
    }

    /// Physical status of input path [`StatusOnDisk::Exists`], [`StatusOnDisk::DoesNotExist`] or [`StatusOnDisk::Unknown`]
    pub(crate) fn status_on_disk(&self) -> StatusOnDisk {
        match self.stop_status() {
            StopStatus::Early(step) => match &step.contents {
                PhysicalNode::File(_)
                | PhysicalNode::Missing(_)
                | PhysicalNode::Symlink {
                    resolved: Ok(_), ..
                } => StatusOnDisk::DoesNotExist,
                PhysicalNode::UnknownLookup(_)
                | PhysicalNode::Raced { .. }
                | PhysicalNode::Symlink {
                    resolved: Err(_), ..
                } => StatusOnDisk::Unknown,
                // Should be unreachable, but types aren't strong enough to prove
                PhysicalNode::ParentDir { .. }
                | PhysicalNode::Directory(_)
                | PhysicalNode::NotReached => {
                    warn(format!(
                        "unexpected early status_on_disk: {:?})",
                        step.contents
                    ));
                    StatusOnDisk::Unknown
                }
                PhysicalNode::ParentNoExec { entry, .. } => {
                    if entry.is_some() {
                        StatusOnDisk::Unknown
                    } else {
                        StatusOnDisk::DoesNotExist
                    }
                }
            },
            StopStatus::Final(step) => match &step.contents {
                PhysicalNode::File(_)
                | PhysicalNode::Directory(_)
                | PhysicalNode::Symlink { .. }
                | PhysicalNode::ParentDir { .. } => StatusOnDisk::Exists,
                PhysicalNode::ParentNoExec { entry, .. } => {
                    if entry.is_some() {
                        StatusOnDisk::Exists
                    } else {
                        StatusOnDisk::DoesNotExist
                    }
                }
                PhysicalNode::Missing(_) => StatusOnDisk::DoesNotExist,
                PhysicalNode::UnknownLookup(_) | PhysicalNode::Raced { .. } => {
                    StatusOnDisk::Unknown
                }
                PhysicalNode::NotReached => {
                    warn("unexpected final status_on_disk: PhysicalNode::NotReached");
                    StatusOnDisk::Unknown
                }
            },
        }
    }

    /// Whether the trace completed or stopped early and the step where it stopped
    pub(crate) fn stop_status(&self) -> StopStatus<'_> {
        let cursor = StepCursor::trace_stopped_at(self);
        let step = cursor.current();

        if cursor.after().is_none() {
            StopStatus::Final(step)
        } else {
            StopStatus::Early(step)
        }
    }

    /// Returns input index of a directory that comes before a component
    pub(crate) fn input_index_of_dir(&self, input: &DirBeforeComponent) -> Option<usize> {
        let dir = &input.dir_before;
        self.steps
            .iter()
            .rev()
            .filter(|step| !matches!(step.contents, PhysicalNode::ParentDir { .. }))
            .find_map(|step| {
                if step.contents.resolved_to()?.as_ref() == dir {
                    step.input_index
                } else {
                    None
                }
            })
    }
}

/// A pointer to a specific step, allows us to inspect up or down
struct StepCursor<'a> {
    steps: &'a NonEmpty<Step>,
    index: usize,
}

impl<'a> StepCursor<'a> {
    /// Builds a cursor pointing at the last component we walked on the file system
    fn trace_stopped_at(trace: &'a Trace) -> Self {
        let steps = &trace.steps;
        let index = steps
            .iter()
            .rposition(|step| !matches!(step.contents, PhysicalNode::NotReached))
            .unwrap_or_else(|| {
                warn(format!(
                    "expected trace of `{}` to reach one physical node on disk but it did not.",
                    trace.input.display()
                ));
                0
            });

        StepCursor { steps, index }
    }

    /// Step we are pointing at
    fn current(&self) -> &'a Step {
        &self.steps[self.index]
    }

    fn before(&self) -> Option<&'a Step> {
        self.index.checked_sub(1).map(|i| &self.steps[i])
    }

    fn after(&self) -> Option<&'a Step> {
        self.steps.get(self.index + 1)
    }
}

/// Represents the status of the trace, if it finished or not.
#[derive(Debug)]
pub(crate) enum StopStatus<'a> {
    /// Stopped before last step
    Early(&'a Step),
    /// Traced to completion (may still have errors in last step)
    Final(&'a Step),
}

/// Status of a path on disk
#[derive(Debug)]
pub(crate) enum StatusOnDisk {
    /// File exists, possibly with problems but it exists
    Exists,
    /// We can prove the file does not exist
    DoesNotExist,
    /// We can neither confirm nor deny the file exists (because we don't have enough evidence)
    Unknown,
}

/// Match each [`Step::input_index`] to the index of the input [`Component`]
fn attribute(steps: &mut [Step], input: &Path) {
    // The walk records no step for a `.`, so skipping them here keeps one in the middle of
    // a path from knocking everything before it out of alignment.
    let components = input
        .components()
        .enumerate()
        .filter(|(_, component)| !matches!(component, Component::CurDir))
        .collect::<Vec<_>>();

    for (step, (index, component)) in steps.iter_mut().rev().zip(components.into_iter().rev()) {
        if step.name.as_ref() != component.as_os_str() {
            break;
        }
        step.input_index = Some(index);
    }
}

/// Current status of an in-progress walk.
enum WalkPosition {
    /// A resolved directory, the only state where the filesystem can be asked anything
    Dir(CanonicalPath),
    /// Names below here are still exact, but nothing is there to examine
    ///
    /// Reached through a proven absence or through something that is not a directory.
    /// Both rule out a symlink, which is all that naming needs.
    Ghost(AbsPath),
    /// Nothing below here can even be named
    Lost,
}

/// Warn instead of panic-ing if we can help it
///
/// Panic strategy:
/// - Prefer to fully handle or hold errors.
/// - Sometimes we must `expect()` for internals of types. Only do this if there's no viable alternative,
///   or if the local logic can **prove** it's safe if the type system cannot.
/// - For cases where "this should never happen" but a sound, default value can be returned: warn (to)
///   and use the default value.
fn warn(input: impl AsRef<str>) {
    // Avoid panic-ing outside of tests since this library is for supplementary information.
    let _ = writeln!(stderr(), "path_facts internal warning: {}", input.as_ref());

    #[cfg(test)]
    panic!("{}", input.as_ref());
}

/// Moves into `name`, which sits inside whatever the walk has reached
fn enter(position: WalkPosition, name: NormalComponent) -> (Step, WalkPosition) {
    match position {
        WalkPosition::Lost => (
            Step {
                input_index: None,
                name: name.into(),
                at: None,
                contents: PhysicalNode::NotReached,
            },
            WalkPosition::Lost,
        ),
        // Nothing exists below a name that does not, so nothing below it can be a symlink
        // and there is nothing left to ask the filesystem.
        WalkPosition::Ghost(ghost) => {
            let at = join(&ghost, &name);
            (
                Step {
                    input_index: None,
                    name: name.into(),
                    at: Some(at.clone()),
                    contents: PhysicalNode::NotReached,
                },
                WalkPosition::Ghost(at),
            )
        }
        WalkPosition::Dir(dir) => {
            let at = join(&AbsPath::from(dir.clone()), &name);
            let (saw, next) = look(&dir, &name, &at);
            (
                Step {
                    input_index: None,
                    name: name.into(),
                    at: Some(at),
                    contents: saw,
                },
                next,
            )
        }
    }
}

/// Asks the filesystem about `name` inside the resolved directory `dir`
fn look(dir: &CanonicalPath, name: &NormalComponent, at: &AbsPath) -> (PhysicalNode, WalkPosition) {
    match dir.entry(name) {
        Ok(Entry::Canonical(child, lstat)) => {
            if lstat.is_dir() {
                (
                    PhysicalNode::Directory(child.clone()),
                    WalkPosition::Dir(child),
                )
            } else {
                (
                    PhysicalNode::File(child.clone()),
                    WalkPosition::Ghost(child.into()),
                )
            }
        }
        Ok(Entry::Symlink) => follow(dir, at),
        // Both readings of a failure here are claims about a name inside a directory, so
        // neither survives the directory having stopped being one. Worth the second look
        // because it costs one `lstat` per walk: the walk never asks the filesystem
        // anything again after this.
        Err(error) => {
            if dir.as_ref().access(AccessMode::EXECUTE).is_err() {
                if let Ok(read_dir) = dir.as_ref().read_dir() {
                    let mut any_errors = false;
                    let mut found = false;
                    for entry in read_dir {
                        match entry {
                            Ok(entry) => {
                                // TODO track case insensitive OS-s and compare here
                                if entry.file_name() == name.as_ref() {
                                    found = true;
                                }
                            }
                            Err(_) => any_errors = true,
                        }
                    }
                    if found {
                        return (
                            PhysicalNode::ParentNoExec {
                                parent: dir.clone(),
                                entry: Some(name.clone()),
                                error,
                            },
                            WalkPosition::Lost,
                        );
                    } else if !any_errors {
                        return (
                            PhysicalNode::ParentNoExec {
                                parent: dir.clone(),
                                entry: None,
                                error,
                            },
                            WalkPosition::Lost,
                        );
                    }
                }
            }

            // We know there's a problem with the directory, check to see if a prior assumption failed to hold
            match check_directory_race(dir) {
                Some(why) => (PhysicalNode::Raced { why, error }, WalkPosition::Lost),
                None => {
                    if error.kind() == ErrorKind::NotFound {
                        (
                            PhysicalNode::Missing(error),
                            WalkPosition::Ghost(at.clone()),
                        )
                    } else {
                        (PhysicalNode::UnknownLookup(error), WalkPosition::Lost)
                    }
                }
            }
        }
    }
}

/// Looks again at a directory the walk already stepped into, once a lookup inside it failed
///
/// Returns `Some()` with a reason for why a data race occured, or None when the path is
/// still a directory.
///
/// Answers the sentence for [`PhysicalNode::Raced`] when the directory is not one any more,
/// which is the case neither reading of that failure can survive. `NotFound` on a name
/// inside a directory and `NotFound` because the directory itself is gone arrive as the
/// same error about the same path, so nothing but a second look separates them.
///
/// Reads the directory rather than the error code, which is what makes it uniform: no
/// mapping from a platform's numbers to a meaning, and the same answer on targets where
/// those numbers differ.
///
/// `lstat` rather than `stat`, because a [`CanonicalPath`] holds no symlinks. A symlink
/// standing where the walk left a directory contradicts that even when it points at a
/// directory, and following it first would hide exactly the swap worth reporting.
///
/// Answers `None` on anything short of a sighting, including a second look that fails for
/// its own reasons. Proving nothing is the honest outcome there, and the walk keeps the
/// reading it already had. That direction is deliberate: a missed contradiction leaves a
/// report no worse than before, while an invented one blames the filesystem for a bug in
/// here.
fn check_directory_race(dir: &CanonicalPath) -> Option<&'static str> {
    match std::fs::symlink_metadata(dir.as_ref()) {
        Ok(lstat) => {
            if lstat.is_dir() {
                None
            } else {
                Some("the walk stepped into the directory holding this name, and lstat now reports something that is not a directory in its place")
            }
        }
        Err(error) => {
            if error.kind() == ErrorKind::NotFound {
                Some("the walk stepped into the directory holding this name, and lstat now reports nothing there")
            } else {
                None
            }
        }
    }
}

/// Resolves a symlink
///
/// Delegates the following to `realpath` so the loop budget and the rest of the target's
/// own chain are libc's problem rather than ours. Whatever it refuses on comes back as the
/// error, unexamined.
fn follow(dir: &CanonicalPath, link: &AbsPath) -> (PhysicalNode, WalkPosition) {
    // `readlink` and `realpath` are two more looks at a name `lstat` already called a
    // symlink, and either can be refused (Apple checks the link's own permission bits on
    // `readlink`). A link whose target cannot be read is a fact about the link, so the
    // refusal is carried on `target` rather than folded into a `Raced` step. Both are kept
    // because "cannot read where it points" and "cannot resolve where it lands" are
    // different failures about the same link.
    let target = readlink(dir, link);

    let resolved = match CanonicalPath::new(link) {
        Ok(resolved) => resolved,
        Err(error) => {
            return (
                PhysicalNode::Symlink {
                    target,
                    resolved: Err(error),
                },
                WalkPosition::Lost,
            )
        }
    };

    // A link to a file cannot hold children. Letting it through would leave the walk
    // standing in something that is not a directory, which would make `..` unsound.
    match std::fs::metadata(resolved.as_ref()) {
        Ok(metadata) => {
            let (landed, next) = if metadata.is_dir() {
                (
                    ResolvedSymlink::Dir(resolved.clone()),
                    WalkPosition::Dir(resolved.clone()),
                )
            } else {
                (
                    ResolvedSymlink::File(resolved.clone()),
                    WalkPosition::Ghost(AbsPath::from(resolved)),
                )
            };
            (
                PhysicalNode::Symlink {
                    target,
                    resolved: Ok(landed),
                },
                next,
            )
        }
        // `realpath` answering means the target exists and every directory on the way to it
        // is searchable, which is everything a `stat` on it needs.
        Err(error) => (
            PhysicalNode::Raced {
                why: "realpath resolved this link, stat failed to resolve",
                error,
            },
            WalkPosition::Lost,
        ),
    }
}

/// Applies `..` to whatever the walk has reached
fn up(position: WalkPosition, name: ParentDirComponent) -> (Step, WalkPosition) {
    match position {
        WalkPosition::Dir(from) => {
            let resolved = from.parent().unwrap_or_else(|| from.clone());
            (
                Step {
                    input_index: None,
                    name: name.into(),
                    at: Some(AbsPath::from(resolved.clone())),
                    contents: PhysicalNode::ParentDir {
                        folded: from,
                        resolved: resolved.clone(),
                    },
                },
                WalkPosition::Dir(resolved),
            )
        }
        // Folding this would assume whatever fills the gap is a directory rather than a
        // symlink pointing somewhere else, which is a guess about the future rather than a
        // fact about now.
        WalkPosition::Ghost(_) | WalkPosition::Lost => (
            Step {
                input_index: None,
                name: name.into(),
                at: None,
                contents: PhysicalNode::NotReached,
            },
            WalkPosition::Lost,
        ),
    }
}

fn join(dir: &AbsPath, name: &NormalComponent) -> AbsPath {
    dir.join_normal(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_unfolded;

    /// Traces a path the way a caller would, anchoring it and handing the spelling back
    fn walk(path: impl AsRef<Path>) -> Trace {
        Trace::new(path.as_ref()).unwrap()
    }

    /// The steps `attribute` sees, without walking anything to produce them
    fn named(names: &[&str]) -> Vec<Step> {
        names
            .iter()
            .map(|name| Step {
                input_index: None,
                name: one(name),
                at: None,
                contents: PhysicalNode::NotReached,
            })
            .collect()
    }

    fn norm(name: &str) -> NormalComponent {
        one(name).normal().unwrap().clone()
    }

    fn one(name: &str) -> OwnedComponent {
        let mut components = Path::new(name).components();
        let first = component::owned(components.next().unwrap());
        if let Some(c) = components.next() {
            panic!(
                "expected name `{}` to be one component, got {:?}, {:?}, {:?}",
                name, first, c, components
            )
        }
        first
    }

    /// Tempdirs on macOS live under a symlink, so resolve once up front to keep the
    /// expected values readable.
    fn tempdir() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().canonicalize().unwrap();
        (temp, dir)
    }

    /// The filesystem root `path` is anchored to
    ///
    /// Built from components so no test has to spell out a separator.
    fn root_of(path: &Path) -> PathBuf {
        path.components()
            .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
            .map(std::path::Component::as_os_str)
            .collect()
    }

    /// The step the walk stopped at, which has to exist for the test to be about anything
    fn stopped(trace: &Trace) -> &Step {
        trace.stopped_early_at().unwrap()
    }

    #[test]
    fn test_entry_that_exists_resolves() {
        let (_temp, dir) = tempdir();
        let path = dir.join("a").join("b").join("c");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();

        let trace = walk(&path);
        assert!(trace.stopped_early_at().is_none());
        assert_eq!(trace.physical_location().unwrap().as_ref(), path);

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir.join("a").join("b"));
        assert_eq!(listing.component, norm("c"));
    }

    /// A name that is not there is still a name in a directory that is, so the listing
    /// answers even though the path does not resolve.
    #[test]
    fn test_entry_that_is_missing_still_has_a_directory() {
        let (_temp, dir) = tempdir();
        let b = dir.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();

        let trace = walk(b.join("c"));
        assert!(trace.physical_location().is_none());
        assert!(matches!(stopped(&trace).contents, PhysicalNode::Missing(_)));

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), b);
        assert_eq!(listing.component, norm("c"));
        assert_eq!(
            trace.last_step_expanded_lexical_parent().unwrap().as_ref(),
            b
        );
    }

    /// A broken symlink is still an entry in its directory, so the listing answers even
    /// though the path cannot be canonicalized.
    #[cfg(unix)]
    #[test]
    fn test_entry_that_is_a_broken_symlink_reports_its_target() {
        let (_temp, dir) = tempdir();
        let target = dir.join("gone");
        std::os::unix::fs::symlink(&target, dir.join("c")).unwrap();

        let trace = walk(dir.join("c"));
        assert!(trace.physical_location().is_none());
        match &stopped(&trace).contents {
            PhysicalNode::Symlink {
                target: to,
                resolved,
            } => {
                let (_, abs) = to.as_ref().unwrap();
                assert_eq!(abs.as_ref(), target);
                assert_eq!(resolved.as_ref().unwrap_err().kind(), ErrorKind::NotFound);
            }
            other => panic!("expected Symlink got {:?}", other),
        }

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir);
        assert_eq!(listing.component, norm("c"));
    }

    /// A circular link is the same shape as a dangling one: `lstat` succeeds on the link
    /// itself, and only following it fails. Nothing here matches on the error code, the
    /// system's own message is the report.
    #[cfg(unix)]
    #[test]
    fn test_entry_that_is_a_symlink_loop_still_has_a_directory() {
        let (_temp, dir) = tempdir();
        std::os::unix::fs::symlink("loop2", dir.join("loop1")).unwrap();
        std::os::unix::fs::symlink("loop1", dir.join("loop2")).unwrap();

        let trace = walk(dir.join("loop1"));
        match &stopped(&trace).contents {
            PhysicalNode::Symlink { target, resolved } => {
                let (_, abs) = target.as_ref().unwrap();
                assert_eq!(abs.as_ref(), dir.join("loop2"));
                assert!(resolved.is_err());
            }
            other => panic!("expected Symlink got {:?}", other),
        }

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir);
        assert_eq!(listing.component, norm("loop1"));
    }

    /// The case a canonical path cannot express: `b` is missing, so `<dir>/a/b` is an exact
    /// name for where `c` would sit even though nothing is there.
    #[test]
    fn test_naming_continues_below_a_missing_name() {
        let (_temp, dir) = tempdir();
        std::fs::create_dir(dir.join("a")).unwrap();
        let b = dir.join("a").join("b");

        let trace = walk(b.join("c"));
        let stop = stopped(&trace);
        assert!(matches!(stop.contents, PhysicalNode::Missing(_)));
        assert_eq!(stop.at.as_ref().unwrap().as_ref(), b);

        // The listing points at the missing name itself, inside a directory that exists
        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir.join("a"));
        assert_eq!(listing.component, norm("b"));
        assert_eq!(
            trace.last_step_expanded_lexical_parent().unwrap().as_ref(),
            b
        );
    }

    /// One absence covers every name below it, no further syscalls needed.
    #[test]
    fn test_naming_continues_arbitrarily_far_below_a_missing_name() {
        let (_temp, dir) = tempdir();
        std::fs::create_dir(dir.join("a")).unwrap();
        let b = dir.join("a").join("b");

        let trace = walk(b.join("c").join("d"));
        assert_eq!(stopped(&trace).at.as_ref().unwrap().as_ref(), b);

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir.join("a"));
        assert_eq!(listing.component, norm("b"));
        assert_eq!(
            trace.last_step_expanded_lexical_parent().unwrap().as_ref(),
            b.join("c")
        );
    }

    /// The case the old `ParentDirAfterHole` verdict made confusing by reporting only what
    /// it could not do. A `..` below a missing name changes nothing about where the walk
    /// stopped, so the listing still answers.
    #[test]
    fn test_dot_dot_below_a_missing_name_still_has_a_directory() {
        let (_temp, dir) = tempdir();
        let b = dir.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();
        let c = b.join("c");

        let trace = walk(join_unfolded(&c, &[".."]));
        let stop = stopped(&trace);
        assert!(matches!(stop.contents, PhysicalNode::Missing(_)));
        assert_eq!(stop.at.as_ref().unwrap().as_ref(), c);

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), b);
        assert_eq!(listing.component, norm("c"));

        // The only query the `..` takes away
        assert!(trace.last_step_expanded_lexical_parent().is_none());
    }

    /// `..` cannot be folded across a hole: doing so would assume `b` gets created as a
    /// directory rather than as a symlink pointing somewhere else.
    #[test]
    fn test_dot_dot_below_a_missing_name_stops_naming() {
        let (_temp, dir) = tempdir();
        std::fs::create_dir(dir.join("a")).unwrap();
        let b = dir.join("a").join("b");

        let trace = walk(join_unfolded(&b, &["..", "c"]));
        assert_eq!(stopped(&trace).at.as_ref().unwrap().as_ref(), b);

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir.join("a"));
        assert_eq!(listing.component, norm("b"));
        assert!(trace.last_step_expanded_lexical_parent().is_none());
    }

    /// Components below a symlink are named inside its target, not below the link.
    #[cfg(unix)]
    #[test]
    fn test_symlink_in_the_middle_resolves_to_its_target() {
        let (_temp, dir) = tempdir();
        let link = dir.join("link");
        let target = dir.join("other");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let trace = walk(link.join("c"));
        assert!(trace.physical_location().is_none());

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), target);
        assert_eq!(listing.component, norm("c"));
        assert_eq!(
            trace.last_step_expanded_lexical_parent().unwrap().as_ref(),
            target
        );

        let trace = walk(link.join("c").join("d").join("e"));
        assert_eq!(
            stopped(&trace).at.as_ref().unwrap().as_ref(),
            target.join("c")
        );

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), target);
        assert_eq!(listing.component, norm("c"));
        assert_eq!(
            trace.last_step_expanded_lexical_parent().unwrap().as_ref(),
            target.join("c").join("d")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_broken_symlink_in_the_middle_reports_its_target() {
        let (_temp, dir) = tempdir();
        let link = dir.join("dangling");
        let target = dir.join("missing");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let trace = walk(link.join("c"));
        let stop = stopped(&trace);
        assert_eq!(stop.at.as_ref().unwrap().as_ref(), link);
        match &stop.contents {
            PhysicalNode::Symlink {
                target: to,
                resolved,
            } => {
                let (_, to) = to.as_ref().unwrap();
                assert_eq!(to.as_ref(), target);
                assert!(resolved.is_err());
            }
            other => panic!("expected Symlink got {:?}", other),
        }

        // Wherever the link points, the link itself sits in a directory that exists
        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir);
        assert_eq!(listing.component, norm("dangling"));

        // A child of the link would live under the target, which does not resolve
        assert!(trace.last_step_expanded_lexical_parent().is_none());
    }

    /// A relative target resolves against the directory holding the link. Joining it onto
    /// the link itself would name `<dir>/dangling/missing`.
    #[cfg(unix)]
    #[test]
    fn test_broken_symlink_with_a_relative_target() {
        let (_temp, dir) = tempdir();
        let link = dir.join("dangling");
        std::os::unix::fs::symlink("missing", &link).unwrap();

        let trace = walk(link.join("c"));
        match &stopped(&trace).contents {
            PhysicalNode::Symlink { target, .. } => {
                let (_, abs) = target.as_ref().unwrap();
                assert_eq!(abs.as_ref(), dir.join("missing"));
            }
            other => panic!("expected Symlink got {:?}", other),
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_readlink_dotdot_on_relative_windows() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let target = PathBuf::from("x").join("y").join("z");
        assert!(target.is_relative());

        std::env::set_current_dir(dir).unwrap();
        std::fs::create_dir_all(&target).unwrap();

        let link = dir.join("link");
        std::os::windows::fs::symlink_dir(&target, &link).unwrap();

        assert!(link.is_symlink());
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());

        let trailing = join_unfolded(&link, &["eaten", ".."]);
        std::fs::create_dir_all(trailing.parent().unwrap()).unwrap();

        let trace = walk(&trailing);
        let recorded = trace
            .steps
            .iter()
            .find_map(|step| match &step.contents {
                PhysicalNode::Symlink {
                    target: Ok((_, abs)),
                    ..
                } => Some(abs.as_ref()),
                _ => None,
            })
            .unwrap();

        assert_eq!(
            recorded.canonicalize().unwrap(),
            link.canonicalize().unwrap(),
        );

        let last = trace.last_step();
        match &last.contents {
            PhysicalNode::ParentDir { resolved, .. } => assert_eq!(
                resolved.as_ref().canonicalize().unwrap(),
                target.canonicalize().unwrap(),
                "the `..` folds onto the symlink target, not onto the link itself",
            ),
            other => panic!(
                "expected the trailing `..` to fold to ParentDir, got {:?}",
                other
            ),
        }
    }

    /// A directory that stops being one partway through a walk cannot be arranged on
    /// demand, so these ask the second look directly what it makes of each way that can
    /// happen. What they cover is the judgement, not the timing.
    ///
    /// Builds the [`CanonicalPath`] first and disturbs the path afterwards, which is the
    /// same order the walk sees: a directory it stood in, and a later look that disagrees.
    fn entered(path: &Path) -> CanonicalPath {
        CanonicalPath::new(&AbsPath::new(path).unwrap()).unwrap()
    }

    #[test]
    fn test_the_second_look_passes_a_directory_that_is_still_there() {
        let (_temp, dir) = tempdir();
        let path = dir.join("d");
        std::fs::create_dir(&path).unwrap();

        assert_eq!(check_directory_race(&entered(&path)), None);
    }

    #[test]
    fn test_the_second_look_catches_a_directory_replaced_by_a_file() {
        let (_temp, dir) = tempdir();
        let path = dir.join("d");
        std::fs::create_dir(&path).unwrap();
        let entered = entered(&path);

        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "").unwrap();

        assert!(check_directory_race(&entered).is_some());
    }

    #[test]
    fn test_the_second_look_catches_a_directory_that_is_gone() {
        let (_temp, dir) = tempdir();
        let path = dir.join("d");
        std::fs::create_dir(&path).unwrap();
        let entered = entered(&path);

        std::fs::remove_dir(&path).unwrap();

        assert!(check_directory_race(&entered).is_some());
    }

    /// Pointing at a directory does not make it the directory the walk entered. A
    /// [`CanonicalPath`] holds no symlinks, so a link standing in its place contradicts
    /// that however it resolves, which is why the second look is an `lstat`.
    #[cfg(unix)]
    #[test]
    fn test_the_second_look_catches_a_directory_replaced_by_a_symlink_to_a_directory() {
        let (_temp, dir) = tempdir();
        let path = dir.join("d");
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(dir.join("elsewhere")).unwrap();
        let entered = entered(&path);

        std::fs::remove_dir(&path).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), &path).unwrap();

        assert!(std::fs::metadata(&path).unwrap().is_dir());
        assert!(check_directory_race(&entered).is_some());
    }

    /// The guarantee that matters most: a second look that cannot see has proven nothing,
    /// and says so. Reporting a contradiction here would blame the filesystem for a walk
    /// that was never shown to be wrong.
    #[cfg(unix)]
    #[test]
    fn test_the_second_look_reports_nothing_when_it_cannot_see() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, dir) = tempdir();
        let closed = dir.join("closed");
        let path = closed.join("d");
        std::fs::create_dir(&closed).unwrap();
        std::fs::create_dir(&path).unwrap();
        let entered = entered(&path);

        // Without execute on the parent, `lstat` on the directory itself is refused
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o600)).unwrap();

        assert!(std::fs::symlink_metadata(&path).is_err());
        assert_eq!(check_directory_race(&entered), None);
    }

    /// Without execute the directory cannot be searched, so a name inside it could be
    /// missing, a directory, or a symlink. Absence is unprovable, so naming stops.
    #[cfg(unix)]
    #[test]
    fn test_directory_without_execute_stops_the_walk() {
        use faccess::{AccessMode, PathExt};
        use std::os::unix::fs::PermissionsExt;

        let (_temp, dir) = tempdir();
        let closed = dir.join("closed");
        std::fs::create_dir(&closed).unwrap();
        std::fs::create_dir(closed.join("inner")).unwrap();
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o600)).unwrap();

        let trace = walk(closed.join("inner").join("leaf"));
        let stop = stopped(&trace);
        assert!(matches!(stop.contents, PhysicalNode::ParentNoExec { .. }));
        assert_eq!(stop.at.as_ref().unwrap().as_ref(), closed.join("inner"));
        assert!(closed.join("inner").access(AccessMode::EXECUTE).is_err());

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), closed);
        assert_eq!(listing.component, norm("inner"));
        assert!(trace.last_step_expanded_lexical_parent().is_none());
    }

    #[test]
    fn test_file_in_the_middle_stops_the_walk() {
        let (_temp, dir) = tempdir();
        std::fs::write(dir.join("f"), "").unwrap();

        let trace = walk(dir.join("f").join("c"));
        match &stopped(&trace).contents {
            PhysicalNode::File(file) => assert_eq!(file.as_ref(), dir.join("f")),
            other => panic!("expected File got {:?}", other),
        }

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir);
        assert_eq!(listing.component, norm("f"));
    }

    /// An ancestor that blocks descent stops the walk early, and that step carries the physical
    /// location and the node the walk found there.
    #[test]
    fn test_early_stop_names_the_file_ancestor_that_blocked_descent() {
        let (_temp, dir) = tempdir();
        std::fs::write(dir.join("f"), "").unwrap();

        let trace = walk(dir.join("f").join("c"));
        let StopStatus::Early(step) = trace.stop_status() else {
            panic!("expected the file below the final component to stop the walk early");
        };
        assert_eq!(step.at.as_ref().unwrap().as_ref(), dir.join("f"));
        assert!(matches!(step.contents, PhysicalNode::File(_)));
    }

    /// A missing name below the final component blocks descent the same way a file does.
    #[test]
    fn test_early_stop_names_a_missing_ancestor() {
        let (_temp, dir) = tempdir();
        std::fs::create_dir(dir.join("a")).unwrap();
        let b = dir.join("a").join("b");

        let trace = walk(b.join("c"));

        let StopStatus::Early(step) = trace.stop_status() else {
            panic!("expected the missing name below the final component to stop the walk early");
        };
        assert_eq!(step.at.as_ref().unwrap().as_ref(), b);
        assert!(matches!(step.contents, PhysicalNode::Missing(_)));
    }

    #[cfg(unix)]
    #[test]
    fn test_early_stop_names_a_broken_symlink_ancestor() {
        let (_temp, dir) = tempdir();
        let link = dir.join("dangling");
        std::os::unix::fs::symlink(dir.join("missing"), &link).unwrap();

        let trace = walk(link.join("c"));
        let StopStatus::Early(step) = trace.stop_status() else {
            panic!("expected the broken symlink below the final component to stop the walk early");
        };
        assert_eq!(step.at.as_ref().unwrap().as_ref(), link);
        match &step.contents {
            PhysicalNode::Symlink { resolved, .. } => assert!(resolved.is_err()),
            other => panic!("expected Symlink got {:?}", other),
        }
    }

    /// A path that resolves all the way to its final component never stopped early, so the
    /// stop is `Final`.
    #[test]
    fn test_stop_is_final_when_the_path_resolves() {
        let (_temp, dir) = tempdir();
        let path = dir.join("a").join("b");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();

        assert!(matches!(walk(&path).stop_status(), StopStatus::Final(_)));
    }

    /// The final component being the problem is the path's own facts, not a prior ancestor's,
    /// so the stop is `Final`, not `Early`.
    #[test]
    fn test_stop_is_final_when_only_the_final_component_is_missing() {
        let (_temp, dir) = tempdir();
        std::fs::create_dir(dir.join("a")).unwrap();

        assert!(matches!(
            walk(dir.join("a").join("missing")).stop_status(),
            StopStatus::Final(_)
        ));
    }

    /// Normalize to linux behavior. Apple's `realpath` answers `<dir>` here, but
    /// `symlink_metadata` errors. Walking left to right follows the kernel on every
    /// platform instead, so the trace never claims a location.
    #[test]
    fn test_dot_dot_on_a_file_stops_the_walk() {
        let (_temp, dir) = tempdir();
        std::fs::write(dir.join("f"), "").unwrap();

        let trace = walk(join_unfolded(&dir, &["f", ".."]));
        assert!(trace.physical_location().is_none());
        match &stopped(&trace).contents {
            PhysicalNode::File(file) => assert_eq!(file.as_ref(), dir.join("f")),
            other => panic!("expected File got {:?}", other),
        }

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir);
        assert_eq!(listing.component, norm("f"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_to_a_file_in_the_middle_stops_the_walk() {
        let (_temp, dir) = tempdir();
        std::fs::write(dir.join("f"), "").unwrap();
        std::os::unix::fs::symlink(dir.join("f"), dir.join("flink")).unwrap();

        let trace = walk(dir.join("flink").join("c"));
        match &stopped(&trace).contents {
            PhysicalNode::Symlink { resolved, .. } => {
                assert_eq!(resolved.as_ref().unwrap().path().as_ref(), dir.join("f"));
            }
            other => panic!("expected Symlink got {:?}", other),
        }

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir);
        assert_eq!(listing.component, norm("flink"));
    }

    /// The entry name comes from the resolved location, so a trailing `..` points at `a`
    /// rather than at the `b` that trimming the last component would produce. This is what
    /// `read_dir` needs: no directory holds an entry called `..`.
    #[test]
    fn test_dot_dot_is_listed_as_the_entry_it_resolves_to() {
        let (_temp, dir) = tempdir();
        let b = dir.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();

        let trace = walk(b.join(".."));
        assert_eq!(trace.physical_location().unwrap().as_ref(), dir.join("a"));

        let listing = trace.listing().unwrap();
        assert_eq!(listing.dir_before.as_ref(), dir);
        assert_eq!(listing.component, norm("a"));
        assert_eq!(
            trace.last_step_expanded_lexical_parent().unwrap().as_ref(),
            dir
        );
    }

    #[test]
    fn test_root_sits_in_nothing() {
        let (_temp, dir) = tempdir();
        let root = root_of(&dir);
        let result = Trace::new(&root);
        match result {
            Err(CannotTrace::IsRoot(path)) => {
                assert_eq!(path.as_ref(), root.canonicalize().unwrap());
            }
            _ => panic!("expected is root err got {:?}", result),
        }
    }

    #[test]
    fn test_dot_dot_at_root_sits_in_nothing() {
        let (_temp, dir) = tempdir();
        let trace = walk(join_unfolded(&root_of(&dir), &[".."]));

        assert_eq!(trace.physical_location().unwrap().as_ref(), root_of(&dir));
        assert!(trace.listing().is_none());
    }

    /// The location is root even though the directory walked to is not.
    ///
    /// Borrows a real top level directory from the canonicalized tempdir rather than
    /// reading one out of the root. Root entries can be symlinks (`/tmp` on macOS points at
    /// `/private/tmp`), and `..` through one of those lands somewhere else entirely.
    /// Components of a canonical path are symlink free by definition.
    #[test]
    fn test_dot_dot_resolving_to_root_sits_in_nothing() {
        let (_temp, dir) = tempdir();
        let first = dir
            .components()
            .find(|component| matches!(component, Component::Normal(_)))
            .unwrap();
        let top = root_of(&dir).join(first.as_os_str());

        let trace = walk(join_unfolded(&top, &[".."]));
        assert_eq!(trace.physical_location().unwrap().as_ref(), root_of(&dir));
        assert!(trace.listing().is_none());
    }

    /// On Unix `components()` strips every `.`, so a path that is lexically nothing but root
    /// and dots reduces to `[RootDir]` and is caught by `IsRoot` before the walk. This is why
    /// the zero-step case never arises here, and it documents the boundary the Windows
    /// verbatim case crosses.
    #[cfg(unix)]
    #[test]
    fn test_root_with_only_dot_components_is_caught_as_is_root() {
        for input in ["/.", "//", "/./.", "/././"] {
            let result = Trace::new(Path::new(input));
            assert!(
                matches!(result, Err(CannotTrace::IsRoot(_))),
                "expected {:?} to be IsRoot, got {:?}",
                input,
                result,
            );
        }
    }

    /// Regression test for `\\?\C:\.`
    ///
    /// This path `\\?\C:\.` is a "verbatim" absolute path with only a `CurDir` (`.`) in it. This
    /// caused us to produce a `Trace` with zero steps.
    #[cfg(windows)]
    #[test]
    fn test_verbatim_root_with_a_dot_does_not_produce_a_zero_step_trace() {
        let verbatim_root = root_of(&std::env::current_dir().unwrap().canonicalize().unwrap());
        let dotted = join_unfolded(&verbatim_root, &["."]);
        assert!(
            dotted.components().any(|c| matches!(c, Component::CurDir)),
            "expected the verbatim `.` to survive in {:?}, got {:?}",
            dotted,
            dotted.components().collect::<Vec<_>>(),
        );

        match Trace::new(&dotted) {
            // Lexically nothing but a root, so it must be rejected exactly like `\\?\C:\`
            Err(CannotTrace::IsRoot(_)) => {}
            // Any other `Ok` must still honor the invariant every accessor depends on
            Ok(trace) => assert!(
                !trace.steps.is_empty(),
                "Trace::new returned Ok with zero steps for {:?}",
                dotted,
            ),
            other => panic!("unexpected result for {:?}: {:?}", dotted, other),
        }
    }

    /// Nothing was prepended, so every step points back at the component it came from.
    #[test]
    fn test_absolute_input_attributes_every_step() {
        let (_temp, dir) = tempdir();
        let path = dir.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        let trace = walk(&path);
        let input = path.components().collect::<Vec<_>>();

        for step in &trace.steps {
            let index = step.input_index.unwrap();
            assert_eq!(input[index].as_os_str(), step.name.as_ref());
        }
    }

    /// The `./hello/lol/foo.txt` case. Only what the caller typed is attributed, and the
    /// directory it was resolved against is still walked, just anonymously.
    #[test]
    fn test_relative_input_attributes_only_what_the_caller_wrote() {
        let (_temp, dir) = tempdir();
        std::env::set_current_dir(&dir).unwrap();
        let input = Path::new(".").join("hello").join("lol").join("foo.txt");

        let trace = walk(&input);
        assert_eq!(trace.input(), input);

        // `.` is component 0 and never becomes a step, so attribution starts at 1
        let attributed = trace
            .steps
            .iter()
            .filter_map(|step| Some((step.input_index?, step.name.clone())))
            .collect::<Vec<_>>();
        assert_eq!(
            attributed,
            vec![(1, one("hello")), (2, one("lol")), (3, one("foo.txt"))]
        );

        // A problem in the anchor is still reportable, just against an absolute path
        // rather than against anything the caller would recognize
        for step in trace.steps.iter().filter(|step| step.input_index.is_none()) {
            assert!(
                step.at.is_some(),
                "{:?} should still name a path",
                step.name
            );
        }
    }

    /// An absolute path is kept verbatim, so a `.` in the middle survives into the walk as
    /// a component with no step. Attribution steps over it rather than giving up on
    /// everything before it.
    #[test]
    fn test_current_dir_part_does_not_break_attribution() {
        let (_temp, dir) = tempdir();
        std::fs::create_dir(dir.join("a")).unwrap();
        let path = dir.join(".").join("a");

        let trace = walk(&path);
        let input = path.components().collect::<Vec<_>>();

        for step in &trace.steps {
            let index = step.input_index.unwrap();
            assert_eq!(input[index].as_os_str(), step.name.as_ref());
        }
    }

    /// Attribution is a guess about two lists lining up, so it is built to lose rather than
    /// to lie.
    ///
    /// [`Trace::new`] derives one list from the other, so this cannot be reached by walking
    /// a path. It is still worth holding: anchoring is [`std::path::absolute`], whose
    /// normalization differs across platforms, and a caret pointed at the wrong component
    /// is worse than no caret at all.
    #[test]
    fn test_attribution_gives_up_rather_than_pointing_at_the_wrong_component() {
        let mut steps = named(&["home", "you", "hello"]);

        attribute(&mut steps, Path::new("unrelated"));

        assert!(steps.iter().all(|step| step.input_index.is_none()));
    }

    /// Alignment runs from the end, so a shared tail is attributed and the first name that
    /// disagrees stops it rather than sending it hunting further up.
    #[test]
    fn test_attribution_keeps_the_tail_it_can_line_up() {
        let mut steps = named(&["home", "you", "hello"]);

        attribute(&mut steps, Path::new("elsewhere/hello"));

        assert_eq!(
            steps
                .iter()
                .map(|step| step.input_index)
                .collect::<Vec<_>>(),
            vec![None, None, Some(1)]
        );
    }

    /// A walk that resolved cleanly caught no contradiction, so there is no race to report.
    #[test]
    fn test_raced_is_none_when_the_walk_did_not_race() {
        let (_temp, dir) = tempdir();
        let path = dir.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        assert!(walk(&path).raced().is_none());
    }

    /// A real race cannot be timed on demand, so `inject_race` writes a known one into an
    /// otherwise real trace. `raced` reports it, and it reads as `Unknown` on disk exactly as a
    /// real race does.
    #[test]
    fn test_inject_race_is_detected_and_reads_as_unknown() {
        let (_temp, dir) = tempdir();
        let path = dir.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        let mut trace = walk(&path);
        assert!(trace.raced().is_none());

        trace.append_race("two calls disagreed", std::io::Error::other("boom"));

        let raced = trace.raced().unwrap();
        match &raced.contents {
            PhysicalNode::Raced { why, error } => {
                assert_eq!(*why, "two calls disagreed");
                assert_eq!(error.to_string(), "boom");
            }
            other => panic!("expected Raced got {:?}", other),
        }
        assert!(matches!(trace.status_on_disk(), StatusOnDisk::Unknown));
    }

    /// A race before the final component leaves the steps after it `NotReached`, so it is not
    /// `last_step`. `raced` scans rather than trusting the last step, which is what lets a
    /// mid-path race still be reported.
    #[test]
    fn test_raced_finds_a_mid_path_race_that_last_step_would_miss() {
        let (_temp, dir) = tempdir();
        std::fs::write(dir.join("f"), "").unwrap();

        // `<dir>/f/c` stops at the file `f`, leaving `c` unreached
        let mut trace = walk(dir.join("f").join("c"));
        assert!(matches!(
            trace.last_step().contents,
            PhysicalNode::NotReached
        ));

        trace.append_race("two calls disagreed", std::io::Error::other("boom"));

        // The race lands on the stopped step, and the last step is still `NotReached`: a
        // check against `last_step` alone would report no race.
        assert!(matches!(
            trace.last_step().contents,
            PhysicalNode::NotReached
        ));
        let raced = trace.raced().unwrap();
        assert!(matches!(raced.contents, PhysicalNode::Raced { .. }));
        assert_eq!(raced.at.as_ref().unwrap().as_ref(), dir.join("f"));
    }
}

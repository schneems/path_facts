//! What the filesystem reported at each component of a path
//!
//! Canonicalizing answers one question ("where does this land") and refuses to answer
//! anything else. When it fails it collapses every reason into a single error, and a
//! `NotFound` from a missing name and a `NotFound` from a broken symlink want different
//! things said about them.
//!
//! A [`Trace`] is the walk itself rather than its verdict: one [`Step`] per component,
//! each holding what a single `lstat` reported there. Nothing in it says whether what was
//! found is good or bad news, because that depends on what the caller was trying to do. A
//! missing path is a bug for `File::open` and a success for `File::create_new`.
//!
//! ## Pointing back at the input
//!
//! Walking requires an absolute path, because there is nowhere to start otherwise, but
//! `./hello` is what the caller typed and `/home/you/hello` is not. So the walk keeps both:
//! [`Trace::input`] is the spelling it was given, and each [`Step::input`] says which
//! component of that spelling it came from.
//!
//! [`Trace::new`] takes the spelling and anchors it, rather than taking the two paths and
//! trusting them to match. A report built on an `input` that was not what got walked
//! points at a path nobody asked about, and the only way to rule that out is to leave no
//! way to say it.
//!
//! Steps from the directory a relative path was resolved against have no such component,
//! and report `None`. They are still worth showing, since the problem may well be up there,
//! but they have to be shown against [`Step::at`] rather than against the input. That is
//! the difference between "cannot stat `./hello`" and "the directory it would sit in is
//! `/home/you`, which is what these permissions belong to".
//!
//! No byte offsets are recorded yet. Component indices into [`Path::components`] of
//! [`Trace::input`] are enough to attach them later, without the walk having to care how
//! a path gets rendered. That iterator is the index space: a raw split on separators is
//! not, because it still sees the `.` parts and extra slashes that `components` drops.
//!
//! The facts the walk is built on:
//!
//! - A path is a lexical representation of parts, this can correspond to a physical location
//!   on disk, but not every lexical operation maps to a physical operation for every path.
//! - Symlinks and parent parts `..` require mapping lexical to physical paths for accurate information.
//! - Sometimes we don't have enough information to confidently say if a path exists or not.
//! - Resolution goes left to right, never by trimming the end. Trimming cannot work: the
//!   last component may be `..`, and any component may be a symlink, so the physical parent of a
//!   path is not generally the path with its last part removed.
//! - `lstat` reporting `NotFound` is what allows naming past it. A name that does not
//!   exist cannot be a symlink, so from that point on appending components is exact. This
//!   is why `/a/b/c` can still name `/a/b` even when `b` is missing.
//! - That name is exact as a *name*, not as an inode. Nothing is there to have permissions
//!   or entries, and it never becomes a [`CanonicalPath`].
//! - A `..` on a physical location can be "folded" i.e. `/a/b/../c` can be folded into
//!   `/a/c` if `a` and `b` exist and are directories. A `..` is a "parent directory"
//!   and the format of `x/y/z` of a path implies that `y` MUST be a directory for it to be
//!   able to hold `z`. Therefore in `x/y/z/..` if `y` does not exist or is a file, it cannot be a directory
//!   and the `..` cannot be folded into a physical location. But it can still be a valid path in
//!   some operations such as `create_dir_all`. We don't know what operation was attempted that caused
//!   someone to ask for `PathFacts` so we must provide helpful information for a range of scenarios.
//! - Losing access (`EACCES`) is the real blocker, not absence. Without a proven absence
//!   there is no way to argue a name is not a symlink.
//! - Every fact is about the moment its syscall ran, and the walk makes several. Where two
//!   of them contradict each other the walk says so ([`PhysicalNode::Raced`]) rather than
//!   picking whichever it saw last, but most changes underneath it are invisible and it
//!   claims no better.
//! - A root is canonical by construction and reachable only by observation. `\\server\share`
//!   names a machine that can be off, and nothing can be said about a path hanging off a
//!   root that does not answer.

// Nothing renders a trace yet. Two consumers are waiting on it: `happy_path::state` builds
// its directory listing from `AbsPath::lex_parent`, which is wrong for a trailing `..`, and
// the `ParentProblem` arm of `PathFacts` re-walks from scratch once per ancestor.

use crate::abs_path::{readlink, AbsPath, AbsPathError, RelativePath};
use crate::canonical_path::{CannotCanonicalizeAnything, CanonicalPath, Entry};
use crate::happy_path::UnknownPath;
use faccess::{AccessMode, PathExt};
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// What a single `lstat` reported at one component
///
/// Describes physical disk. Whether any of it is a problem is the caller's question, not this
/// type's.
#[derive(Debug)]
pub(crate) enum PhysicalNode {
    /// A directory, so the walk could continue through it
    Directory(CanonicalPath),

    /// Not a directory and not a symlink, so nothing can be entered below it
    File(CanonicalPath),

    /// A symlink
    ///
    /// Pointing somewhere and arriving somewhere are separate facts, so both are kept.
    /// `target` is a single `readlink`, reporting where the link points without following
    /// it. `resolved` is where following it lands, and carries the error when it lands
    /// nowhere.
    ///
    /// Both are results because `lstat` calling a name a symlink does not promise that
    /// either call succeeds. Apple's libc checks the link's own permission bits on
    /// `readlink` while `lstat` needs only search on the parent, so a link can read as a
    /// symlink and still refuse to say where it points. That is a fact about the link, not
    /// a contradiction in the walk: `target` carries the refusal rather than
    /// [`PhysicalNode::Raced`] swallowing the whole step.
    ///
    /// The error is kept rather than classified. A circular link, a dangling one, and one
    /// whose target is unreadable are all "this link did not resolve, and here is what the
    /// system said about it", which spares us matching on error codes to reword what the
    /// system already worded.
    ///
    /// A link that does not resolve is reported rather than chased: the target's own chain
    /// may not resolve either, and symlinks can be made to point in a circle.
    Symlink {
        target: Result<AbsPath, std::io::Error>,
        resolved: Result<CanonicalPath, std::io::Error>,
    },

    /// Nothing is at this name
    ///
    /// The one observation that proves the name is not a symlink, which is what lets the
    /// walk keep naming components below it. Carries the error so a report can quote the
    /// system rather than invent wording for it.
    Missing(#[allow(dead_code)] std::io::Error),

    /// Can list entries from the parent but cannot access them directly
    ParentNoExec {
        parent: CanonicalPath,
        /// Some if the directory has the entry, otherwise None
        entry: Option<OsString>,
    },

    /// Cannot tell missing from directory from symlink here
    ///
    /// A denial proves nothing either way, so naming stops.
    Denied(#[allow(dead_code)] std::io::Error),

    /// `..` moved from one resolved directory to another
    ///
    /// Safe to apply lexically: a [`CanonicalPath`] holds no symlinks, and POSIX defines
    /// `..` on a symlink free path as removing the last component. Root is its own parent,
    /// matching `realpath` on `/..`.
    Up {
        #[allow(dead_code)]
        from: CanonicalPath,
        to: CanonicalPath,
    },

    /// Two calls about this component contradicted each other
    ///
    /// Not a fact about the path, a fact about the walk. Something moved underneath it, or
    /// the filesystem answered inconsistently, and the second answer ruled out what the
    /// first one had established. Nothing below it can be trusted, so naming stops.
    ///
    /// `why` names the pair of calls that disagreed. It is written at the site rather than
    /// derived from the error, so a bug report can quote a sentence that leads a maintainer
    /// straight back to the code that produced it.
    ///
    /// This catches self-contradiction only, which is a small corner of what can go wrong
    /// underneath a walk. The walk notices a directory it stood in going away, because it
    /// held a claim about that directory to be contradicted. It cannot notice the same
    /// thing happening to the name it was asked about: a file deleted a moment before the
    /// `lstat` reads as "was never there", which is exactly what [`PhysicalNode::Missing`]
    /// would say if it were true, and there is no earlier observation to weigh it against.
    ///
    /// So no `Raced` step means none was caught, not that nothing moved.
    Raced {
        #[allow(dead_code)]
        why: &'static str,
        #[allow(dead_code)]
        error: std::io::Error,
    },

    /// Never examined, because an earlier component stopped the walk
    NotReached,
}

impl PhysicalNode {
    /// The physical location this component resolved to
    ///
    /// `None` for every observation that is not a place: an absence, a denial, a link that
    /// goes nowhere, a contradiction, and a component the walk never got to.
    pub(crate) fn resolved_to(&self) -> Option<Cow<'_, CanonicalPath>> {
        match self {
            PhysicalNode::Directory(path) | PhysicalNode::File(path) => Some(Cow::Borrowed(path)),
            PhysicalNode::Symlink { resolved, .. } => resolved.as_ref().ok().map(Cow::Borrowed),
            PhysicalNode::Up { to, .. } => Some(Cow::Borrowed(to)),
            PhysicalNode::Missing(_)
            | PhysicalNode::Denied(_)
            | PhysicalNode::Raced { .. }
            | PhysicalNode::NotReached => None,
            PhysicalNode::ParentNoExec { parent, entry } => entry
                .as_ref()
                .map(|entry| unsafe { Cow::Owned(parent.unchecked_join(entry)) }),
        }
    }
}

/// One component of a path, and what was found there
#[derive(Debug)]
pub(crate) struct Step {
    /// Which component of the caller's path this came from
    ///
    /// Indexes [`Trace::input`] by [`Path::components`], so a report can point back at
    /// the part of the path the caller actually typed. `None` for a component the caller
    /// did not write, which is every component of the directory a relative path was
    /// resolved against. Those are worth reporting too, they just have to be shown against
    /// an absolute path ([`Step::at`]) rather than against the input.
    ///
    /// An interior `.` is not a component, so it has no index and cannot be a span. That
    /// does not hide a problem: `.` is a no-op, and `/a/./b` still attributes `b` as
    /// component 2. The later caret has to use this same iterator. A `split('/')` of the
    /// original bytes would call index 2 `.` and underline the wrong place.
    pub(crate) input: Option<usize>,

    /// One component of the walked path, never the whole path
    ///
    /// For `/home/you/hello.txt` this is `home`, then `you`, then `hello.txt`. The physical
    /// location those name is [`Step::at`]; the spelling the caller typed is
    /// [`Trace::input`].
    ///
    /// Taken from [`Path::components`] on the anchored path: a file or directory name, or
    /// `..`. Root is skipped, and so is `.`. [`Path::components`] already drops `.` except
    /// at the start of a relative path, so `/a/./b` and `/a/b` produce the same names, and
    /// a trailing `/a/b/.` is just `/a/b`. A leading `./hello` still yields a
    /// [`Component::CurDir`] that the walk skips, so this is never `"."`. A dot *inside* a
    /// name is ordinary (`foo.txt`, `.bashrc`).
    ///
    /// ```text
    /// /home/you/hello.txt  →  home, you, hello.txt
    /// ./hello/lol/foo.txt  →  hello, lol, foo.txt   (plus unattributed CWD parts)
    /// /a/./b               →  a, b
    /// /a/b/.               →  a, b
    /// /a/b/..              →  a, b, ..
    /// .bashrc              →  .bashrc
    /// ```
    ///
    /// When a relative path is resolved, the directory it was anchored to is walked first.
    /// Those steps have names too, they just have no [`Step::input`].
    pub(crate) name: OsString,

    /// The path this component names
    ///
    /// Physical rather than lexical: a component below a symlink is named inside the
    /// link's target, and `..` names the directory it moved to.
    ///
    /// `None` once naming stops, which happens two ways. A `..` that cannot be folded
    /// leaves nothing to name, and so does any component below a denial, a loop, or a link
    /// that goes nowhere. Naming survives a proven absence, because absence rules out a
    /// symlink.
    #[allow(dead_code)]
    pub(crate) at: Option<AbsPath>,

    pub(crate) contents: PhysicalNode,
}

/// A directory that can be listed, and the name to point at inside it
#[derive(Debug)]
pub(crate) struct Listing {
    pub(crate) dir: CanonicalPath,
    pub(crate) entry: OsString,
}

impl From<CannotTrace> for UnknownPath {
    fn from(value: CannotTrace) -> Self {
        match value {
            CannotTrace::Anchor(error) => UnknownPath::AbsPathError(error),
            CannotTrace::Root(error) => UnknownPath::CannotCanonicalizeAnything(error),
        }
    }
}

impl From<CannotTrace> for Box<UnknownPath> {
    fn from(value: CannotTrace) -> Self {
        match value {
            CannotTrace::Anchor(error) => Box::new(UnknownPath::AbsPathError(error)),
            CannotTrace::Root(error) => Box::new(UnknownPath::CannotCanonicalizeAnything(error)),
        }
    }
}

/// Every component of a path, and what the filesystem reported at each
#[derive(Debug)]
pub(crate) struct Trace {
    /// The path the caller passed in, before it was anchored
    ///
    /// Kept verbatim, separators and `.` parts and all, because it is the spelling the
    /// caller recognizes. Anchoring rewrites `./hello` into `/home/you/hello`, which is
    /// the right thing to walk and the wrong thing to show back to someone who typed the
    /// first one.
    input: PathBuf,

    /// The same path, anchored so the walk has somewhere to start
    ///
    /// Not a different path, a different spelling of the same one. Every step's
    /// [`Step::at`] is built from this rather than from [`Trace::input`], because `./hello`
    /// names nothing on its own.
    absolute: AbsPath,

    /// The filesystem root every step hangs off of
    ///
    /// Proven before the walk starts rather than assumed. A root is canonical by
    /// construction, but that says nothing about whether it can be reached: a Windows path
    /// can name a share on a machine that is off, and there is nothing to say about
    /// `\\server\share\a\b\c` when `\\server\share` itself does not answer.
    root: CanonicalPath,
    steps: Vec<Step>,
}

/// The walk could not begin
///
/// Distinct from everything else this module reports. A problem at a component is a fact
/// about that component: it is recorded, the walk stops, and the steps before it still
/// answer. Neither of these leaves anything to record.
#[derive(Debug)]
pub(crate) enum CannotTrace {
    /// The spelling could not be anchored, so there is nowhere to start walking from
    Anchor(AbsPathError),

    /// The root the path hangs off did not answer
    Root(CannotCanonicalizeAnything),
}

impl Trace {
    /// Walks the path left to right, recording what is at each component
    ///
    /// Takes the spelling the caller wrote and anchors it here, rather than accepting an
    /// already-anchored path alongside it. The two have to agree for a report to point at
    /// anything real, and deriving one from the other is what leaves no way for them to
    /// disagree. [`Trace::absolute`] hands the anchored path back, so a caller that needs
    /// it does not have to anchor a second time.
    ///
    /// Errors only when the walk cannot begin, which is the one shape of failure that
    /// leaves nothing to report. Every other problem stops the walk partway and still
    /// answers about the components before it, while a [`Trace`] of nothing would invite
    /// queries that could only say `None` without saying why.
    pub(crate) fn new(input: &Path) -> Result<Trace, CannotTrace> {
        let absolute = AbsPath::new(input).map_err(CannotTrace::Anchor)?;

        let root = absolute.lex_root();
        let root = CanonicalPath::new(&root).map_err(|root_error| {
            CannotTrace::Root(CannotCanonicalizeAnything {
                original: absolute.clone(),
                root,
                root_error,
            })
        })?;

        let mut position = Reached::Dir(root.clone());
        let mut steps = Vec::new();

        for component in absolute.as_ref().components() {
            let (step, next) = match component {
                // Already where the walk starts, and `components` only ever yields these at
                // the front of an absolute path.
                Component::Prefix(_) | Component::RootDir => continue,
                // `components` normally drops `.`, but it keeps them behind a verbatim
                // prefix (`\\?\`), which is what Windows canonicalization hands back. So
                // this arm does real work there rather than only guarding against a stray
                // dot in an absolute path.
                //
                // Treating `.` as a no-op is more permissive than Windows itself: a
                // verbatim path skips OS normalization, so `\\?\C:\a\.\b` does not name
                // `\\?\C:\a\b` to the kernel. Posix semantics win here, matching the rest
                // of this library.
                Component::CurDir => continue,
                Component::Normal(name) => enter(position, name),
                Component::ParentDir => up(position, component.as_os_str()),
            };

            steps.push(step);
            position = next;
        }

        attribute(&mut steps, input);

        Ok(Trace {
            input: input.to_path_buf(),
            absolute,
            root,
            steps,
        })
    }

    /// Where the path lands, when every component resolved
    ///
    /// `Some` exactly when `Trace::stopped_early_at` is `None`.
    ///
    #[cfg(test)]
    pub(crate) fn physical_location(&self) -> Option<CanonicalPath> {
        if self.stopped_early_at().is_some() {
            return None;
        }

        match self.steps.last() {
            // Nothing but a root, which the walk proved before it started
            None => Some(self.root.clone()),
            Some(last) => last.contents.resolved_to().map(Cow::into_owned),
        }
    }

    // The step of the last part of the input path
    pub(crate) fn last_step(&self) -> &Step {
        self.steps
            .last()
            .expect("TODO this is wrong, steps can be empty if only contains root")
    }

    /// The step the walk could not continue past
    ///
    /// `None` exactly when every component resolved, which is when `Trace::physical_location`
    /// answers. Every step after this one is [`PhysicalNode::NotReached`], so this single
    /// observation explains all of them.
    #[cfg(test)]
    pub(crate) fn stopped_early_at(&self) -> Option<&Step> {
        let index = self.examined()?;
        let step = &self.steps[index];

        // Arriving at a file, a link, or an absence is an answer when the path ends there,
        // and a dead end when more components follow.
        let stopped = index + 1 != self.steps.len() || step.contents.resolved_to().is_none();
        stopped.then_some(step)
    }

    /// The directory to list, and the name to point at inside it
    ///
    /// Always an existing directory: for every arm except `Up` it is a place the walk
    /// resolved; for `Up` it is the lexical parent of a canonical path, which must exist.
    /// Existence, being-a-directory, and search (execute) permission are proven — the walk
    /// or the canonicalization that produced the path traversed through it. Read permission
    /// is not proven, so `read_dir` on it can still fail with `EACCES`.
    ///
    /// When the walk stopped this is the deepest directory it reached plus the name that
    /// failed there; otherwise it is the directory holding the final component.
    ///
    /// `None` only when the path resolves to a root, which sits in nothing.
    pub(crate) fn listing(&self) -> Option<Listing> {
        let index = self.examined()?;
        let step = &self.steps[index];

        // `..` is not an entry in any directory listing, so name the location it moved to
        // rather than the two dots that were written.
        if let PhysicalNode::Up { to, .. } = &step.contents {
            return Some(Listing {
                dir: to.parent()?,
                entry: to.as_ref().file_name()?.to_os_string(),
            });
        }

        let dir = match index.checked_sub(1) {
            None => self.root.clone(),
            Some(previous) => self.steps[previous]
                .contents
                .resolved_to()
                .expect("the walk only examines a component from a directory it resolved")
                .into_owned(),
        };

        Some(Listing {
            dir,
            entry: step.name.clone(),
        })
    }

    /// The name of the directory the final component sits in
    ///
    /// Exact as a *name*, which is weaker than [`Trace::listing`]: the answer may be a
    /// directory that does not exist, so it cannot be listed and has no permissions. What
    /// it is good for is saying which directory would have to appear.
    ///
    /// `None` when naming stopped, and when the path is a root.
    #[cfg(test)]
    pub(crate) fn parent_name(&self) -> Option<AbsPath> {
        self.steps.last()?.at.as_ref()?.lex_parent()
    }

    /// The path the caller passed in, before it was anchored
    #[allow(dead_code)]
    pub(crate) fn input(&self) -> &Path {
        &self.input
    }

    /// The anchored path the walk was actually run against
    ///
    /// Held so a caller that needs it can take this one rather than anchor a second time
    /// and end up with two paths that could differ.
    pub(crate) fn absolute(&self) -> &AbsPath {
        &self.absolute
    }

    /// Index of the last component the filesystem was actually asked about
    ///
    /// Once the walk leaves a resolved directory it never returns to one, so this is the
    /// point everything after it hangs off of.
    ///
    /// Returns None if steps is empty (when root)
    fn examined(&self) -> Option<usize> {
        self.steps
            .iter()
            .rposition(|step| !matches!(step.contents, PhysicalNode::NotReached))
    }

    /// Reports on the physical status of the input path
    ///
    /// - Exists: Input maps to a file at that location, but there may be other problems
    /// - DoesNotExist: Input definitively does NOT exist due to an observation made on a prior path
    /// - Unknown: Problems prevent us from conclusively saying if the path is exists or not
    pub(crate) fn status_on_disk(&self) -> StatusOnDisk {
        match self.stop_status() {
            StopStatus::Root => StatusOnDisk::Exists,
            StopStatus::Early(step) => match &step.contents {
                PhysicalNode::File(_)
                | PhysicalNode::Missing(_)
                | PhysicalNode::Symlink {
                    resolved: Ok(_), ..
                } => StatusOnDisk::DoesNotExist,
                PhysicalNode::Denied(_)
                | PhysicalNode::Raced { .. }
                | PhysicalNode::Symlink {
                    resolved: Err(_), ..
                } => StatusOnDisk::Unknown,
                PhysicalNode::Up { .. } => unreachable!("cannot stop on `..` mid-path"),
                PhysicalNode::Directory(_) => unreachable!("cannot stop on a directory"),
                PhysicalNode::NotReached => unreachable!("stopped node must be reached"),
                PhysicalNode::ParentNoExec { parent: _, entry } => {
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
                | PhysicalNode::Up { .. } => StatusOnDisk::Exists,
                PhysicalNode::ParentNoExec { parent: _, entry } => {
                    if entry.is_some() {
                        StatusOnDisk::Exists
                    } else {
                        StatusOnDisk::DoesNotExist
                    }
                }
                PhysicalNode::Missing(_) => StatusOnDisk::DoesNotExist,
                PhysicalNode::Denied(_) | PhysicalNode::Raced { .. } => StatusOnDisk::Unknown,
                PhysicalNode::NotReached => unreachable!("stopped node must be reached"),
            },
        }
    }

    pub(crate) fn stop_status(&self) -> StopStatus<'_> {
        if self.steps.is_empty() {
            StopStatus::Root
        } else {
            if self.steps.len() - 1 == self.examined().expect("not root") {
                StopStatus::Final(&self.steps[self.steps.len() - 1])
            } else {
                StopStatus::Early(&self.steps[self.examined().expect("not root")])
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum StopStatus<'a> {
    Root,
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

/// Points each step back at the component of `input` it came from
///
/// Aligned from the end, by name. The two lists are not parallel from the front: anchoring
/// a relative path prepends components the caller never wrote, and it drops the `.` parts
/// they did write. They do share a tail, because anchoring only ever prepends.
///
/// Matching names rather than counting is what makes this safe. Anchoring is
/// [`std::path::absolute`], whose normalization differs across platforms, and a
/// disagreement here should cost a step its attribution rather than point a future caret
/// at the wrong component. So the walk stops at the first name that does not line up.
fn attribute(steps: &mut [Step], input: &Path) {
    // The walk records no step for a `.`, so skipping them here keeps one in the middle of
    // a path from knocking everything before it out of alignment.
    let components = input
        .components()
        .enumerate()
        .filter(|(_, component)| !matches!(component, Component::CurDir))
        .collect::<Vec<_>>();

    for (step, (index, component)) in steps.iter_mut().rev().zip(components.into_iter().rev()) {
        if step.name != component.as_os_str() {
            break;
        }
        step.input = Some(index);
    }
}

/// Where the walk has gotten to partway through a path
enum Reached {
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

/// Moves into `name`, which sits inside whatever the walk has reached
fn enter(position: Reached, name: &OsStr) -> (Step, Reached) {
    let name = name.to_os_string();

    match position {
        Reached::Lost => (
            Step {
                input: None,
                name,
                at: None,
                contents: PhysicalNode::NotReached,
            },
            Reached::Lost,
        ),
        // Nothing exists below a name that does not, so nothing below it can be a symlink
        // and there is nothing left to ask the filesystem.
        Reached::Ghost(ghost) => {
            let at = join(&ghost, &name);
            (
                Step {
                    input: None,
                    name,
                    at: Some(at.clone()),
                    contents: PhysicalNode::NotReached,
                },
                Reached::Ghost(at),
            )
        }
        Reached::Dir(dir) => {
            let at = join(&AbsPath::from(dir.clone()), &name);
            let (saw, next) = look(&dir, &name, &at);
            (
                Step {
                    input: None,
                    name,
                    at: Some(at),
                    contents: saw,
                },
                next,
            )
        }
    }
}

/// Asks the filesystem about `name` inside the resolved directory `dir`
fn look(dir: &CanonicalPath, name: &OsStr, at: &AbsPath) -> (PhysicalNode, Reached) {
    match dir.entry(name) {
        Ok(Entry::Canonical(child, lstat)) => {
            if lstat.is_dir() {
                (PhysicalNode::Directory(child.clone()), Reached::Dir(child))
            } else {
                (
                    PhysicalNode::File(child.clone()),
                    Reached::Ghost(child.into()),
                )
            }
        }
        Ok(Entry::Symlink) => follow(at),
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
                                if entry.file_name() == name {
                                    found = true
                                }
                            }
                            Err(_) => any_errors = true,
                        }
                    }
                    if found {
                        return (
                            PhysicalNode::ParentNoExec {
                                parent: dir.clone(),
                                entry: Some(name.to_os_string()),
                            },
                            Reached::Lost,
                        );
                    } else if !any_errors {
                        return (
                            PhysicalNode::ParentNoExec {
                                parent: dir.clone(),
                                entry: None,
                            },
                            Reached::Lost,
                        );
                    }
                }
            }

            match no_longer_a_directory(dir) {
                Some(why) => (PhysicalNode::Raced { why, error }, Reached::Lost),
                None if error.kind() == std::io::ErrorKind::NotFound => {
                    (PhysicalNode::Missing(error), Reached::Ghost(at.clone()))
                }
                None => (PhysicalNode::Denied(error), Reached::Lost),
            }
        }
    }
}

/// Looks again at a directory the walk already stepped into, once a lookup inside it failed
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
fn no_longer_a_directory(dir: &CanonicalPath) -> Option<&'static str> {
    match std::fs::symlink_metadata(dir.as_ref()) {
        Ok(lstat) if lstat.is_dir() => None,
        Ok(_) => Some("the walk stepped into the directory holding this name, and lstat now reports something that is not a directory in its place"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Some("the walk stepped into the directory holding this name, and lstat now reports nothing there")
        }
        Err(_) => None,
    }
}

/// Resolves a symlink
///
/// Delegates the following to `realpath` so the loop budget and the rest of the target's
/// own chain are libc's problem rather than ours. Whatever it refuses on comes back as the
/// error, unexamined.
fn follow(link: &AbsPath) -> (PhysicalNode, Reached) {
    // `readlink` and `realpath` are two more looks at a name `lstat` already called a
    // symlink, and either can be refused (Apple checks the link's own permission bits on
    // `readlink`). A link whose target cannot be read is a fact about the link, so the
    // refusal is carried on `target` rather than folded into a `Raced` step. Both are kept
    // because "cannot read where it points" and "cannot resolve where it lands" are
    // different failures about the same link.
    let target = readlink(link);

    let resolved = match CanonicalPath::new(link) {
        Ok(resolved) => resolved,
        Err(error) => {
            return (
                PhysicalNode::Symlink {
                    target,
                    resolved: Err(error),
                },
                Reached::Lost,
            )
        }
    };

    // A link to a file cannot hold children. Letting it through would leave the walk
    // standing in something that is not a directory, which would make `..` unsound.
    match std::fs::metadata(resolved.as_ref()) {
        Ok(metadata) => {
            let next = if metadata.is_dir() {
                Reached::Dir(resolved.clone())
            } else {
                Reached::Ghost(AbsPath::from(resolved.clone()))
            };
            (
                PhysicalNode::Symlink {
                    target,
                    resolved: Ok(resolved),
                },
                next,
            )
        }
        // `realpath` answering means the target exists and every directory on the way to it
        // is searchable, which is everything a `stat` on it needs.
        Err(error) => (
            PhysicalNode::Raced {
                why: "realpath resolved this link, stat on what it resolved to failed",
                error,
            },
            Reached::Lost,
        ),
    }
}

/// Applies `..` to whatever the walk has reached
fn up(position: Reached, name: &OsStr) -> (Step, Reached) {
    let name = name.to_os_string();

    match position {
        Reached::Dir(from) => {
            let to = from.parent().unwrap_or_else(|| from.clone());
            (
                Step {
                    input: None,
                    name,
                    at: Some(AbsPath::from(to.clone())),
                    contents: PhysicalNode::Up {
                        from,
                        to: to.clone(),
                    },
                },
                Reached::Dir(to),
            )
        }
        // Folding this would assume whatever fills the gap is a directory rather than a
        // symlink pointing somewhere else, which is a guess about the future rather than a
        // fact about now.
        Reached::Ghost(_) | Reached::Lost => (
            Step {
                input: None,
                name,
                at: None,
                contents: PhysicalNode::NotReached,
            },
            Reached::Lost,
        ),
    }
}

fn join(dir: &AbsPath, name: &OsStr) -> AbsPath {
    dir.join_relative(&RelativePath::new(name).expect("a file name is a relative path"))
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
                input: None,
                name: OsString::from(name),
                at: None,
                contents: PhysicalNode::NotReached,
            })
            .collect()
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
            .map(|component| component.as_os_str())
            .collect()
    }

    /// The directory to list and the name to point at, as plain values
    fn listing(trace: &Trace) -> (PathBuf, OsString) {
        let listing = trace.listing().expect("a listing");
        (listing.dir.as_ref().to_path_buf(), listing.entry)
    }

    /// The step the walk stopped at, which has to exist for the test to be about anything
    fn stopped(trace: &Trace) -> &Step {
        trace
            .stopped_early_at()
            .expect("TODO this is wrong can be None when root")
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
        assert_eq!(listing(&trace), (dir.join("a").join("b"), "c".into()));
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
        assert_eq!(listing(&trace), (b.clone(), "c".into()));
        assert_eq!(trace.parent_name().unwrap().as_ref(), b);
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
                assert_eq!(to.as_ref().unwrap().as_ref(), target);
                assert_eq!(
                    resolved.as_ref().unwrap_err().kind(),
                    std::io::ErrorKind::NotFound
                );
            }
            other => panic!("expected Symlink got {:?}", other),
        }
        assert_eq!(listing(&trace), (dir, "c".into()));
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
                assert_eq!(target.as_ref().unwrap().as_ref(), dir.join("loop2"));
                assert!(resolved.is_err());
            }
            other => panic!("expected Symlink got {:?}", other),
        }
        assert_eq!(listing(&trace), (dir, "loop1".into()));
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
        assert_eq!(listing(&trace), (dir.join("a"), "b".into()));
        assert_eq!(trace.parent_name().unwrap().as_ref(), b);
    }

    /// One absence covers every name below it, no further syscalls needed.
    #[test]
    fn test_naming_continues_arbitrarily_far_below_a_missing_name() {
        let (_temp, dir) = tempdir();
        std::fs::create_dir(dir.join("a")).unwrap();
        let b = dir.join("a").join("b");

        let trace = walk(b.join("c").join("d"));
        assert_eq!(stopped(&trace).at.as_ref().unwrap().as_ref(), b);
        assert_eq!(listing(&trace), (dir.join("a"), "b".into()));
        assert_eq!(trace.parent_name().unwrap().as_ref(), b.join("c"));
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
        assert_eq!(listing(&trace), (b, "c".into()));

        // The only query the `..` takes away
        assert!(trace.parent_name().is_none());
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
        assert_eq!(listing(&trace), (dir.join("a"), "b".into()));
        assert!(trace.parent_name().is_none());
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
        assert_eq!(listing(&trace), (target.clone(), "c".into()));
        assert_eq!(trace.parent_name().unwrap().as_ref(), target);

        let trace = walk(link.join("c").join("d").join("e"));
        assert_eq!(
            stopped(&trace).at.as_ref().unwrap().as_ref(),
            target.join("c")
        );
        assert_eq!(listing(&trace), (target.clone(), "c".into()));
        assert_eq!(
            trace.parent_name().unwrap().as_ref(),
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
                assert_eq!(to.as_ref().unwrap().as_ref(), target);
                assert!(resolved.is_err());
            }
            other => panic!("expected Symlink got {:?}", other),
        }

        // Wherever the link points, the link itself sits in a directory that exists
        assert_eq!(listing(&trace), (dir, "dangling".into()));

        // A child of the link would live under the target, which does not resolve
        assert!(trace.parent_name().is_none());
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
                assert_eq!(target.as_ref().unwrap().as_ref(), dir.join("missing"))
            }
            other => panic!("expected Symlink got {:?}", other),
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

        assert_eq!(no_longer_a_directory(&entered(&path)), None);
    }

    #[test]
    fn test_the_second_look_catches_a_directory_replaced_by_a_file() {
        let (_temp, dir) = tempdir();
        let path = dir.join("d");
        std::fs::create_dir(&path).unwrap();
        let entered = entered(&path);

        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "").unwrap();

        assert!(no_longer_a_directory(&entered).is_some());
    }

    #[test]
    fn test_the_second_look_catches_a_directory_that_is_gone() {
        let (_temp, dir) = tempdir();
        let path = dir.join("d");
        std::fs::create_dir(&path).unwrap();
        let entered = entered(&path);

        std::fs::remove_dir(&path).unwrap();

        assert!(no_longer_a_directory(&entered).is_some());
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
        assert!(no_longer_a_directory(&entered).is_some());
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
        assert_eq!(no_longer_a_directory(&entered), None);
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

        assert_eq!(listing(&trace), (closed, "inner".into()));
        assert!(trace.parent_name().is_none());
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
        assert_eq!(listing(&trace), (dir, "f".into()));
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
        assert_eq!(listing(&trace), (dir, "f".into()));
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
                assert_eq!(resolved.as_ref().unwrap().as_ref(), dir.join("f"))
            }
            other => panic!("expected Symlink got {:?}", other),
        }
        assert_eq!(listing(&trace), (dir, "flink".into()));
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
        assert_eq!(listing(&trace), (dir.clone(), "a".into()));
        assert_eq!(trace.parent_name().unwrap().as_ref(), dir);
    }

    #[test]
    fn test_root_sits_in_nothing() {
        let (_temp, dir) = tempdir();
        let trace = walk(root_of(&dir));

        assert_eq!(trace.physical_location().unwrap().as_ref(), root_of(&dir));
        assert!(trace.listing().is_none());
        assert!(trace.parent_name().is_none());
    }

    #[test]
    fn test_dot_dot_at_root_sits_in_nothing() {
        let (_temp, dir) = tempdir();
        let trace = walk(root_of(&dir).join(".."));

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
            .expect("a tempdir lives below at least one top level directory");
        let top = root_of(&dir).join(first.as_os_str());

        let trace = walk(top.join(".."));
        assert_eq!(trace.physical_location().unwrap().as_ref(), root_of(&dir));
        assert!(trace.listing().is_none());
    }

    /// Nothing was prepended, so every step points back at the component it came from.
    #[test]
    fn test_absolute_input_attributes_every_step() {
        let (_temp, dir) = tempdir();
        let path = dir.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        let trace = walk(&path);
        let input = path.components().collect::<Vec<_>>();

        for step in trace.steps {
            let index = step.input.expect("the caller wrote every component");
            assert_eq!(input[index].as_os_str(), step.name);
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
            .filter_map(|step| Some((step.input?, step.name.clone())))
            .collect::<Vec<_>>();
        assert_eq!(
            attributed,
            vec![
                (1, "hello".into()),
                (2, "lol".into()),
                (3, "foo.txt".into())
            ]
        );

        // A problem in the anchor is still reportable, just against an absolute path
        // rather than against anything the caller would recognize
        for step in trace.steps.iter().filter(|step| step.input.is_none()) {
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

        for step in trace.steps {
            let index = step.input.expect("only a `.` went unattributed");
            assert_eq!(input[index].as_os_str(), step.name);
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

        assert!(steps.iter().all(|step| step.input.is_none()));
    }

    /// Alignment runs from the end, so a shared tail is attributed and the first name that
    /// disagrees stops it rather than sending it hunting further up.
    #[test]
    fn test_attribution_keeps_the_tail_it_can_line_up() {
        let mut steps = named(&["home", "you", "hello"]);

        attribute(&mut steps, Path::new("elsewhere/hello"));

        assert_eq!(
            steps.iter().map(|step| step.input).collect::<Vec<_>>(),
            vec![None, None, Some(1)]
        );
    }
}

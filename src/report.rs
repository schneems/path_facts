//! The callout rendering of a path walk.
//!
//! [`Report`] holds the [`Display`] information for [`PathFacts`](crate::PathFacts).

use crate::abs_path::AbsPathError;
use crate::callout::{self, Callout, Caret, Entry, Fact};
use crate::canonical_path::{CannotCanonicalizeAnything, CanonicalPath};
use crate::style;
use crate::trace::{CannotTrace, PhysicalNode, Resolved, StatusOnDisk, Step, StopStatus, Trace};
use faccess::{AccessMode, PathExt};
use std::fmt::Display;
use std::path::{Path, PathBuf};

/// Shows facts about a path as a series of caret-annotated callouts when `Display`ed.
///
/// Same walk and same facts as [`PathFacts`](crate::PathFacts), different presentation. Pick one
/// per call site; they are not meant to be interleaved.
pub(crate) struct Report {
    /// Original input path, exactly as the caller spelled it
    path: PathBuf,
    /// Detected state of the path, paired with the callouts built from it
    trace: Result<(Trace, Vec<Callout>), CannotTrace>,
}

impl Report {
    pub(crate) fn new(path: impl AsRef<Path>) -> Self {
        let result = Trace::new(path.as_ref());
        Self::from_trace_result(path, result)
    }

    /// A report over a walk the caller already has, for a state the filesystem cannot be asked
    /// to produce on demand.
    pub(crate) fn from_trace_result(
        path: impl AsRef<Path>,
        result: Result<Trace, CannotTrace>,
    ) -> Self {
        let trace = result.map(|trace| traced(path.as_ref(), trace));
        Report {
            path: path.as_ref().to_path_buf(),
            trace,
        }
    }
}

impl Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.render().trim_end_matches('\n'))
    }
}

impl Report {
    fn render(&self) -> String {
        let (trace, callouts) = match self.trace.as_ref() {
            Ok(walked) => walked,
            Err(cannot) => return self.render_cannot_trace(cannot),
        };

        format!(
            "{}\n{}",
            self.summary(trace),
            callout::render_callouts(callouts)
        )
    }

    /// The standalone verdict line. No arrow, so a caller may prepend arbitrary text.
    fn summary(&self, trace: &Trace) -> String {
        let input = self.path.display();
        match trace.status_on_disk() {
            StatusOnDisk::Exists => format!("exists `{input}`"),
            StatusOnDisk::DoesNotExist => format!("does not exist `{input}`"),
            StatusOnDisk::Unknown => format!("`{input}`"),
        }
    }

    /// The walk never started, so there are no components to point a caret at.
    fn render_cannot_trace(&self, cannot: &CannotTrace) -> String {
        let input = self.path.display();
        match cannot {
            CannotTrace::Anchor(AbsPathError::PathIsEmpty(path)) => {
                format!("path `{}` is empty", path.display())
            }
            CannotTrace::Anchor(AbsPathError::CannotReadCWD(path, error)) => {
                let callout = Callout {
                    path: path.display().to_string(),
                    caret: None,
                    facts: vec![Fact::Text(format!(
                        "Cannot read the current directory: {error}"
                    ))],
                };
                format!("`{input}`\n{}", callout::render_callout(&callout))
            }
            CannotTrace::IsRoot(root) => {
                if self.path == root.as_ref() {
                    format!("is root {root}")
                } else {
                    format!("is root `{input}` → {root}")
                }
            }
            CannotTrace::RootNotReachable(CannotCanonicalizeAnything {
                original,
                root,
                root_error,
            }) => {
                let mut facts = vec![
                    Fact::Text(format!("Root {root} is not reachable")),
                    Fact::Text(format!("Cannot canonicalize: {root_error}")),
                ];
                if self.path.is_relative() {
                    facts.push(Fact::Text(format!("Absolute {original}")));
                }
                let callout = Callout {
                    path: self.path.display().to_string(),
                    caret: callout::highlight(&self.path, Caret::Last),
                    facts,
                };
                format!("`{input}`\n{}", callout::render_callout(&callout))
            }
        }
    }
}

/// Pairs a walk with the callouts built from it.
///
/// Every filesystem call this type makes happens here: building the callouts reads permissions
/// and lists a directory. Leaving that in `Display` would hide syscalls behind a `{}` and let two
/// renderings of the same value disagree, each having asked disk at a different moment.
fn traced(path: &Path, trace: Trace) -> (Trace, Vec<Callout>) {
    let callouts = callouts(path, &trace);
    (trace, callouts)
}

/// Each callout is a path with associated facts about it.
///
/// Always at least one: the walk always stopped somewhere, and that somewhere is what the first
/// callout describes. The second appears whenever the stopping step sits in a directory the walk
/// managed to look inside.
fn callouts(path: &Path, trace: &Trace) -> Vec<Callout> {
    let (step, early) = match trace.stop_status() {
        StopStatus::Early(step) => (step, true),
        StopStatus::Final(step) => (step, false),
    };

    let mut callouts = vec![stopped_step_callout(path, trace, step, early)];
    if let Some(parent) = parent_of_stopped_callout(path, trace) {
        callouts.push(parent);
    }
    callouts
}

/// The caller's whole path, caret on the component the walk stopped at.
fn stopped_step_callout(path: &Path, trace: &Trace, step: &Step, early: bool) -> Callout {
    let mut facts = vec![Fact::Text(stopped_step_type_line(step, early))];
    facts.extend(stopped_step_detail_facts(step));

    // A relative input names nothing on its own, so show what it was anchored to. Absolute
    // inputs already say it.
    if path.is_relative() {
        facts.push(Fact::Text(format!("Absolute {}", trace.absolute())));
    }

    // Only worth a line when the component landed somewhere other than where it is spelled,
    // which is what a symlink (or a folded `..`) does.
    if let Some(canonical) = resolved_elsewhere(step) {
        facts.push(Fact::Text(format!("Canonical {canonical}")));
    }

    Callout {
        path: path.display().to_string(),
        caret: caret_at(path, step.input),
        facts,
    }
}

/// The caller's whole path again, caret slid back to the directory holding the stopped step.
///
/// Falls back to the absolute parent path when that directory has no component in the input
/// — a relative path whose stopped step sits directly in the current directory has nothing
/// to point at, and neither does one sitting in the root.
fn parent_of_stopped_callout(path: &Path, trace: &Trace) -> Option<Callout> {
    let listing = trace.listing()?;
    let dir = listing.dir;
    let sought = listing.entry;
    let spelled = trace.stop_parent();

    let (shown, caret) = match trace
        .parent_input_index(&dir)
        .and_then(|index| caret_at(path, Some(index)))
    {
        Some(span) => (path.display().to_string(), Some(span)),
        None => {
            let fallback = spelled.as_ref().map_or_else(
                || dir.as_ref().to_path_buf(),
                |parent| parent.as_ref().to_path_buf(),
            );
            let span = callout::highlight(&fallback, Caret::Last);
            (fallback.display().to_string(), span)
        }
    };

    let mut facts = vec![Fact::Text(format!("Dir {}", triad(dir.as_ref())))];

    // The directory as spelled and the directory it resolves to differ whenever a symlink or
    // a `..` sits above the stopped step. Saying so is the only way a reader can tell which
    // directory the listing below describes.
    if spelled.map_or(true, |parent| parent.as_ref() != dir.as_ref()) {
        facts.push(Fact::Text(format!("Canonical {dir}")));
    }

    facts.extend(dir_permission_facts(dir.as_ref()));

    let entries = dir.normal_entries();
    match &entries {
        Err(error) => facts.push(Fact::Text(format!("Cannot list directory: {error}"))),
        // Promoted to its own fact rather than left as a gap in the tree, because a name that
        // is not there cannot be pointed at inside a list of names that are.
        Ok(entries) if !entries.contains(&sought) => facts.push(Fact::Text(format!(
            "❌ Missing `{}`",
            Path::new(&sought).display()
        ))),
        Ok(_) => {}
    }

    // Last, so the tree it nests never comes between two `↳` facts.
    if let Ok(entries) = &entries {
        facts.push(Fact::Contains {
            entries: entries
                .iter()
                .map(|entry| Entry {
                    name: Path::new(entry).display().to_string(),
                    annotation: (entry == &sought).then(|| "(exists)".to_string()),
                })
                .collect(),
        });
    }

    Some(Callout {
        path: shown,
        caret,
        facts,
    })
}

/// A caret over the input component at `index`, falling back to the last component when the
/// step was never attributed to one the caller wrote.
fn caret_at(path: &Path, index: Option<usize>) -> Option<callout::Span> {
    match index {
        Some(index) => callout::highlight(path, Caret::Index(index)),
        None => callout::highlight(path, Caret::Last),
    }
}

/// The one line that always leads the stopping-step callout: what is at that component.
///
/// An early stop folds the reason the walk could not continue into the type itself, because a
/// bare `File` on a component with more path after it reads like an answer rather than the
/// problem it is.
fn stopped_step_type_line(step: &Step, early: bool) -> String {
    let not_a_dir = if early { ", not a dir" } else { "" };

    match &step.contents {
        PhysicalNode::Directory(canonical) => format!("Dir {}", triad(canonical.as_ref())),
        PhysicalNode::File(canonical) => {
            format!("File{not_a_dir} {}", triad(canonical.as_ref()))
        }
        PhysicalNode::ParentDir { resolved, .. } => format!("Dir {}", triad(resolved.as_ref())),
        PhysicalNode::Symlink {
            resolved: Ok(Resolved::Dir(canonical)),
            ..
        } => format!("Symlink, resolves to dir {}", triad(canonical.as_ref())),
        PhysicalNode::Symlink {
            resolved: Ok(Resolved::File(canonical)),
            ..
        } => format!(
            "Symlink, resolves to file{not_a_dir} {}",
            triad(canonical.as_ref())
        ),
        PhysicalNode::Symlink {
            resolved: Err(error),
            ..
        } => format!("Symlink, cannot follow: {error}"),
        PhysicalNode::Missing(error) => format!("Missing: {error}"),
        PhysicalNode::UnknownLookup(error) => format!("Cannot lstat: {error}"),
        PhysicalNode::ParentNoExec { .. } => {
            "Parent directory missing execute permission".to_string()
        }
        PhysicalNode::Raced { why, error } => format!(
            "⚠️ Filesystem change detected while gathering facts.\n\
             ⚠️ Facts displayed may be invalid, stale or disagree.\n\
             ⚠️\n\
             ⚠️ Detected: {why}\n\
             ⚠️ Error: {error}"
        ),
        PhysicalNode::NotReached => {
            unreachable!("the stopping step is by definition one the walk reached")
        }
    }
}

/// Facts that only some node kinds carry: where a link points, and why the walk could not reach
/// through to the name.
fn stopped_step_detail_facts(step: &Step) -> Vec<Fact> {
    match &step.contents {
        PhysicalNode::Symlink { target, .. } => match target {
            Ok((written, absolute)) if absolute.as_ref() == written.as_path() => {
                vec![Fact::Text(format!("Target `{}`", written.display()))]
            }
            Ok((written, absolute)) => vec![Fact::Text(format!(
                "Target `{}` → {absolute}",
                written.display()
            ))],
            Err(error) => vec![Fact::Text(format!("Cannot readlink: {error}"))],
        },
        // The type line says the name was only ever seen in a listing. This says what the system
        // answered when the walk asked about it directly, which is the sentence a reader can
        // search for. The error is the one the walk already holds rather than a fresh call: asking
        // again at render time would put a filesystem read inside `Display`, and an answer from a
        // different moment than the rest of the callout.
        PhysicalNode::ParentNoExec { error, .. } => {
            vec![Fact::Text(format!("Cannot lstat: {error}"))]
        }
        _ => Vec::new(),
    }
}

/// Where the component resolved to, when that is somewhere other than where it is spelled.
///
/// `None` when the two agree, which is the common case and would only add a line repeating the
/// bullet above it.
fn resolved_elsewhere(step: &Step) -> Option<CanonicalPath> {
    match &step.contents {
        // In the `/path/to/directory/a/b/..` is always shown as `/path/to/directory/a`
        PhysicalNode::ParentDir {
            folded: _,
            resolved,
        } => Some(resolved.clone()),
        // A link whose target lands where it points was named by `Target` above.
        PhysicalNode::Symlink {
            target: Ok((_, absolute)),
            resolved: Ok(landed),
        } => (!absolute.same_place_as(landed.path().as_ref())).then(|| landed.path().clone()),
        _ => {
            let resolved = step.contents.resolved_to()?;
            let spelled = step.at.as_ref()?;
            (AsRef::<Path>::as_ref(resolved.as_ref()) != AsRef::<Path>::as_ref(spelled))
                .then(|| resolved.into_owned())
        }
    }
}

/// What a directory's permission bits allow, spelled out, when at least one of them is denied.
///
/// Silent on a directory that permits everything, where the triad above already says so and three
/// more lines of "yes" would bury the facts worth reading.
///
/// Each line stands on its own. It is tempting to fold them together — "entries can be listed but
/// not reached" says the interesting part of two bits in one breath — but that sentence is only
/// true when read is granted, and a `0o111` directory grants search and refuses the listing. A
/// fact that quietly depends on another bit is one a reader cannot check against the triad, so
/// each bit gets its own sentence and claims nothing about the others.
///
/// Read is reported either way, because "the listing you are about to see is everything that is
/// there" and "there is no listing" are both answers the reader needs once anything else is
/// denied. Write and execute appear only when denied: a permission that is granted causes no
/// surprise worth a line.
fn dir_permission_facts(dir: &Path) -> Vec<Fact> {
    let write = dir.access(AccessMode::WRITE).is_ok();
    let execute = dir.access(AccessMode::EXECUTE).is_ok();
    if write && execute {
        return Vec::new();
    }

    // Ordered read, write, execute so they can be read straight down against the triad above.
    let mut facts = vec![Fact::Text(if dir.access(AccessMode::READ).is_ok() {
        "Entries can be listed (read permission)".to_string()
    } else {
        "Cannot list entries (no read permission)".to_string()
    })];

    if !write {
        facts.push(Fact::Text(
            "Cannot create, delete, or rename entries (no write permission)".to_string(),
        ));
    }
    if !execute {
        facts.push(Fact::Text(
            "Cannot enter, traverse, or access entries (no execute permission)".to_string(),
        ));
    }
    facts
}

/// Effective read/write/execute for `path`, inherited permissions included.
fn triad(path: &Path) -> String {
    style::permissions(
        path.access(AccessMode::READ).is_ok(),
        path.access(AccessMode::WRITE).is_ok(),
        path.access(AccessMode::EXECUTE).is_ok(),
    )
}

/// Every scenario the library renders, asserted once.
///
/// [`PathFacts`](crate::PathFacts) forwards its `Display` here, so these snapshots are the only
/// copy. `path_facts::tests` keeps a single test proving that forwarding is verbatim, which is
/// what makes each snapshot below a claim about both types without a second suite to keep in step.
///
/// Every snapshot ends in `🛑` so a trailing newline is visible rather than trimmed away, and
/// every fixture is built under [`Fixture::root`] so the carets have stable names to point at.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_unfolded;
    use crate::test_support::*;

    /// Renders a report and scrubs it with `scrubber`.
    fn report(scrubber: &Scrubber, path: impl AsRef<Path>) -> String {
        scrubber.carets(&Report::new(path).to_string())
    }

    /// Rendering asks disk nothing: every fact is gathered by `Report::new`.
    ///
    /// Deleting everything the callouts describe is the only way to tell the difference from the
    /// outside. A `Display` that still read permissions or listed the directory would answer
    /// from the emptied disk and stop matching the first render.
    #[test]
    fn test_render_does_not_touch_the_filesystem() {
        let fixture = Fixture::new();
        let missing = fixture.join("does_not_exist.txt");
        std::fs::write(fixture.join("sibling.txt"), "").unwrap();

        let report = Report::new(&missing);
        let before = fixture.scrub().carets(&report.to_string());

        std::fs::remove_dir_all(fixture.root()).unwrap();

        assert_eq!(before, fixture.scrub().carets(&report.to_string()));
        assert!(
            before.contains("sibling.txt"),
            "expected the pre-deletion listing to survive into the render:\n{}",
            before
        );
    }

    // A `0o111` (search-only, no-read) parent directory: the walk can *search* through it to
    // resolve `child.txt`, so the path reaches its final component, but the parent cannot be
    // `read_dir`'d. The refused listing is reported on the parent callout itself, which is why the
    // callout names a directory and then declines to show what is in it. Unix-only:
    // search-without-read is a POSIX mode the Windows runner cannot reproduce.
    #[test]
    #[cfg(unix)]
    fn test_prior_dir_problem_search_only_parent() {
        let fixture = Fixture::new();
        let search_only = fixture.join("search_only");
        std::fs::create_dir(&search_only).unwrap();
        let child = search_only.join("child.txt");
        std::fs::write(&child, "").unwrap();

        set_mode(&search_only, 0o111).unwrap(); // execute (searchable) but not readable

        let output = report(&fixture.scrub(), &child);

        // Restore permissions so the tempdir can be cleaned up.
        set_mode(&search_only, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/search_only/child.txt`
         - `/path/to/directory/search_only/child.txt`
                                           ^^^^^^^^^
                                           ↳ File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/search_only/child.txt`
                               ^^^^^^^^^^^
                               ↳ Dir [❌ read, ❌ write, ✅ execute]
                               ↳ Cannot list entries (no read permission)
                               ↳ Cannot create, delete, or rename entries (no write permission)
                               ↳ Cannot list directory: Permission denied (os error 13)
        🛑
        ");
    }

    // A `0o000` parent directory, which is neither searchable nor readable. The lookup inside it
    // is refused with `EACCES` rather than answered with `ENOENT`, so the walk cannot say whether
    // the name is there. The fallback that would normally turn a refused lookup into `Parent
    // directory missing execute permission` needs a listing to check the name against, and this
    // directory will not give one either, so the refusal stands as its own outcome.
    //
    // `child.txt` is written before the mode drops, so this snapshot is a claim about a file that
    // demonstrably exists: `Cannot tell what is here` is the walk declining to guess, not a
    // roundabout spelling of missing.
    //
    // Unix-only, for the same reason as the tests above: the Windows runner's admin token bypasses
    // the denial.
    #[test]
    #[cfg(unix)]
    fn test_lookup_refused_by_an_unreadable_parent() {
        let fixture = Fixture::new();
        let denied = fixture.join("denied");
        std::fs::create_dir(&denied).unwrap();
        let child = denied.join("child.txt");
        std::fs::write(&child, "").unwrap();

        set_mode(&denied, 0o000).unwrap(); // neither readable nor searchable

        let output = report(&fixture.scrub(), &child);

        // Restore permissions so the tempdir can be cleaned up.
        set_mode(&denied, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        `/path/to/directory/denied/child.txt`
         - `/path/to/directory/denied/child.txt`
                                      ^^^^^^^^^
                                      ↳ Cannot lstat: Permission denied (os error 13)
         - `/path/to/directory/denied/child.txt`
                               ^^^^^^
                               ↳ Dir [❌ read, ❌ write, ❌ execute]
                               ↳ Cannot list entries (no read permission)
                               ↳ Cannot create, delete, or rename entries (no write permission)
                               ↳ Cannot enter, traverse, or access entries (no execute permission)
                               ↳ Cannot list directory: Permission denied (os error 13)
        🛑
        ");
    }

    /// Recorded to a file, unlike every other snapshot here, because two other tests read this one
    /// back: the module docs of `lib.rs` and the README generated from them both paste this
    /// rendering, and each proves its paste against
    /// `src/snapshots/prior_dir_problem_is_file.snap`. An inline snapshot lives in the source of
    /// this function, where `include_str!` cannot reach it.
    #[test]
    fn test_prior_dir_problem_is_file() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();

        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::with_settings!({prepend_module_to_snapshot => false}, {
            insta::assert_snapshot!(
                "prior_dir_problem_is_file",
                report(&fixture.scrub(), path)
            );
        });
    }

    #[test]
    fn test_prior_dir_problem_does_not_exist() {
        let fixture = Fixture::new();
        let path = fixture
            .join("a")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        let scrubber = fixture.scrub().not_found(fixture.join("a"));
        insta::assert_snapshot!(report(&scrubber, path), @r"
        does not exist `/path/to/directory/a/b/c/does_not_exist.txt`
         - `/path/to/directory/a/b/c/does_not_exist.txt`
                               ^
                               ↳ Missing: {error}
         - `/path/to/directory/a/b/c/does_not_exist.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `a`
                     ↳ Contains (0)
                       └── (empty)
        🛑
        ");
    }

    #[test]
    fn test_empty_path() {
        insta::assert_snapshot!(Scrubber::new().plain(&Report::new("").to_string()), @r"
        path `` is empty
        🛑
        ");
    }

    #[test]
    fn test_file_exists_is_file() {
        let fixture = Fixture::new();
        let path = fixture.join("exists.txt");
        std::fs::write(&path, "").unwrap();

        insta::assert_snapshot!(report(&fixture.scrub(), path), @r"
        exists `/path/to/directory/exists.txt`
         - `/path/to/directory/exists.txt`
                               ^^^^^^^^^^
                               ↳ File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/exists.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `exists.txt` (exists)
        🛑
        ");
    }

    /// Every shape `resolved_elsewhere` tells apart, in one table.
    ///
    /// The question is not where the component landed — [`PhysicalNode::resolved_to`] answers that
    /// by itself, and every arm here takes the place from there. It is whether a `Canonical` line
    /// would name somewhere the callout has not already named. What changes per arm is the spelling
    /// that counts as "already named": the caret's own location for most nodes, the directory that
    /// held the two dots for a `..`, and the link's written target for a symlink. Weighing all
    /// three against `Step::at` would answer "no" for `..` every time and "yes" for a symlink every
    /// time, which is why the comparison cannot be shared.
    #[test]
    fn test_resolved_elsewhere_per_node_kind() {
        use std::path::Component;

        let fixture = Fixture::new();

        // Somewhere for a `..` to move to.
        let b = fixture.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();

        std::fs::write(fixture.join("file.txt"), "").unwrap();

        // `chain` → `link` → `real`, each target written relative so the walk has to absolutize it
        // before the two can be compared, plus one link that points at nothing.
        std::fs::write(fixture.join("real"), "").unwrap();
        symlink_file("real", fixture.join("link")).unwrap();
        symlink_file("link", fixture.join("chain")).unwrap();
        symlink_file("nowhere", fixture.join("broken")).unwrap();

        // A directory reachable under a name that does not sit beside it, so a `..` out of it lands
        // somewhere a lexical reading of the path would not predict.
        let nested = fixture.join("nest").join("real_dir");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("inside.txt"), "").unwrap();
        symlink_dir(&nested, fixture.join("dir_link")).unwrap();

        // `<root>/..`, the one `..` that cannot move: root is its own parent.
        let root_dot_dot = {
            let root = fixture
                .root()
                .components()
                .take_while(|component| {
                    matches!(component, Component::Prefix(_) | Component::RootDir)
                })
                .map(|component| component.as_os_str())
                .collect::<PathBuf>();
            join_unfolded(&root, &[".."])
        };

        let cases: Vec<(&str, PathBuf)> = vec![
            ("a file spelled where it lives", fixture.join("file.txt")),
            ("a name that is not there", fixture.join("missing.txt")),
            (
                "a file the walk stopped early on",
                fixture.join("file.txt").join("b"),
            ),
            ("`..` that moved", join_unfolded(&b, &[".."])),
            ("`..` at root, which is its own parent", root_dot_dot),
            (
                "`..` out of a symlinked directory",
                join_unfolded(&fixture.join("dir_link"), &[".."]),
            ),
            ("a symlink onto its written target", fixture.join("link")),
            ("a symlink onto another symlink", fixture.join("chain")),
            ("a symlink that resolves nowhere", fixture.join("broken")),
            (
                "a file below a symlinked directory",
                fixture.join("dir_link").join("inside.txt"),
            ),
        ];

        let table = cases
            .iter()
            .map(|(what, path)| {
                let trace = Trace::new(path).expect("a path the walk can start on");
                let (StopStatus::Early(step) | StopStatus::Final(step)) = trace.stop_status();
                let answer = match resolved_elsewhere(step) {
                    Some(canonical) => canonical.to_string(),
                    None => "None".to_string(),
                };
                format!("{what}\n  `{}`\n  → {answer}", path.display())
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        // `root_of` so the `..` at root row reads `/` rather than the drive the tempdir landed on.
        insta::assert_snapshot!(fixture.scrub().root_of(fixture.root()).plain(&table), @r"
        a file spelled where it lives
          `/path/to/directory/file.txt`
          → None

        a name that is not there
          `/path/to/directory/missing.txt`
          → None

        a file the walk stopped early on
          `/path/to/directory/file.txt/b`
          → None

        `..` that moved
          `/path/to/directory/a/b/..`
          → `/path/to/directory/a`

        `..` at root, which is its own parent
          `/..`
          → `/`

        `..` out of a symlinked directory
          `/path/to/directory/dir_link/..`
          → `/path/to/directory/nest`

        a symlink onto its written target
          `/path/to/directory/link`
          → None

        a symlink onto another symlink
          `/path/to/directory/chain`
          → `/path/to/directory/real`

        a symlink that resolves nowhere
          `/path/to/directory/broken`
          → None

        a file below a symlinked directory
          `/path/to/directory/dir_link/inside.txt`
          → None
        🛑
        ");
    }

    /// `<dir>/a/b/..` resolves to `<dir>/a`, so the directory to list is `<dir>` — two steps back
    /// from the `..`, not one. The parent caret cannot point at `b`, which names somewhere else,
    /// so the callout falls back to spelling the directory out. See `Trace::parent_input_index`.
    #[test]
    fn test_exists_dot_dot_annotates_resolved_entry() {
        let fixture = Fixture::new();
        let b = fixture.join("a").join("b");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("inside.txt"), "").unwrap();
        std::fs::write(fixture.join("other.txt"), "").unwrap();

        insta::assert_snapshot!(report(&fixture.scrub(), join_unfolded(&b, &[".."])), @r"
        exists `/path/to/directory/a/b/..`
         - `/path/to/directory/a/b/..`
                                   ^^
                                   ↳ Dir [✅ read, ✅ write, ✅ execute]
                                   ↳ Canonical `/path/to/directory/a`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `a` (exists)
                       └── `other.txt`
        🛑
        ");
    }

    #[test]
    fn test_parent_exists_missing_file() {
        let fixture = Fixture::new();
        let missing = fixture.join("does_not_exist.txt");

        let scrubber = fixture.scrub().not_found(&missing);
        insta::assert_snapshot!(report(&scrubber, &missing), @r"
        does not exist `/path/to/directory/does_not_exist.txt`
         - `/path/to/directory/does_not_exist.txt`
                               ^^^^^^^^^^^^^^^^^^
                               ↳ Missing: {error}
         - `/path/to/directory/does_not_exist.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `does_not_exist.txt`
                     ↳ Contains (0)
                       └── (empty)
        🛑
        ");
    }

    /// Two reports in one message. The summary line carries no `→` arrow precisely so a caller can
    /// prepend `From path` / `To path` and have it still read as a sentence.
    #[test]
    fn test_rename_two_missing_paths() {
        use indoc::formatdoc;

        let fixture = Fixture::new();
        fixture.enter();

        let from = Path::new("doesnotexist.txt");
        let to = Path::new("also_does_not_exist.txt");

        let error = std::fs::rename(from, to)
            .map_err(|_error| {
                formatdoc! {"
                    cannot rename from `{}` to `{}` due to: {{error}}.

                    From path {from_facts}
                    To path {to_facts}
                    ",
                    from.display(),
                    to.display(),
                    from_facts = Report::new(from),
                    to_facts = Report::new(to)
                }
            })
            .unwrap_err();

        // Both reports quote the same refusal, so one rule covers the pair.
        let scrubber = fixture.scrub().not_found(from);
        insta::assert_snapshot!(scrubber.carets(&error), @r"
        cannot rename from `doesnotexist.txt` to `also_does_not_exist.txt` due to: {error}.

        From path does not exist `doesnotexist.txt`
         - `doesnotexist.txt`
            ^^^^^^^^^^^^^^^^
            ↳ Missing: {error}
            ↳ Absolute `/path/to/directory/doesnotexist.txt`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `doesnotexist.txt`
                     ↳ Contains (0)
                       └── (empty)

        To path does not exist `also_does_not_exist.txt`
         - `also_does_not_exist.txt`
            ^^^^^^^^^^^^^^^^^^^^^^^
            ↳ Missing: {error}
            ↳ Absolute `/path/to/directory/also_does_not_exist.txt`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `also_does_not_exist.txt`
                     ↳ Contains (0)
                       └── (empty)

        🛑
        ");
    }

    #[test]
    fn test_relative_path_exists() {
        let fixture = Fixture::new();
        fixture.enter();

        let path = Path::new("exists.txt");
        std::fs::write(path, "").unwrap();

        insta::assert_snapshot!(report(&fixture.scrub(), path), @r"
        exists `exists.txt`
         - `exists.txt`
            ^^^^^^^^^^
            ↳ File [✅ read, ✅ write, ❌ execute]
            ↳ Absolute `/path/to/directory/exists.txt`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `exists.txt` (exists)
        🛑
        ");
    }

    #[test]
    fn test_symlink_to_file() {
        // Two tempdirs guarantee the link and its target share no prefix on any platform.
        let target = Fixture::named("target");
        let link = Fixture::named("link");

        let target_file = target.join("target.txt");
        std::fs::write(&target_file, "content").unwrap();

        let symlink_path = link.join("link_to_target.txt");
        symlink_file(&target_file, &symlink_path).unwrap();

        let scrubber = link.scrub().path(target.anchor(), "");
        insta::assert_snapshot!(report(&scrubber, &symlink_path), @r"
        exists `/path/to/link/link_to_target.txt`
         - `/path/to/link/link_to_target.txt`
                          ^^^^^^^^^^^^^^^^^^
                          ↳ Symlink, resolves to file [✅ read, ✅ write, ❌ execute]
                          ↳ Target `/path/to/target/target.txt`
         - `/path/to/link/link_to_target.txt`
                     ^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `link_to_target.txt` (exists)
        🛑
        ");
    }

    /// The link resolves, so the type line can say what it landed on, and what it landed on is
    /// why the walk stopped. `, not a dir` is the same contrast a plain `File` draws when the
    /// caller wrote more path after it, said here about the destination rather than the link.
    #[test]
    fn test_symlink_to_file_with_more_path_after_it() {
        let target = Fixture::named("target");
        let link = Fixture::named("link");

        let target_file = target.join("target.txt");
        std::fs::write(&target_file, "content").unwrap();

        let symlink_path = link.join("link_to_target.txt");
        symlink_file(&target_file, &symlink_path).unwrap();

        let scrubber = link.scrub().path(target.anchor(), "");
        insta::assert_snapshot!(report(&scrubber, symlink_path.join("below.txt")), @r"
        does not exist `/path/to/link/link_to_target.txt/below.txt`
         - `/path/to/link/link_to_target.txt/below.txt`
                          ^^^^^^^^^^^^^^^^^^
                          ↳ Symlink, resolves to file, not a dir [✅ read, ✅ write, ❌ execute]
                          ↳ Target `/path/to/target/target.txt`
         - `/path/to/link/link_to_target.txt/below.txt`
                     ^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `link_to_target.txt` (exists)
        🛑
        ");
    }

    #[test]
    fn test_symlink_to_directory() {
        let target = Fixture::named("target");
        let link = Fixture::named("link");

        let target_dir = target.join("target_dir");
        std::fs::create_dir(&target_dir).unwrap();

        let symlink_path = link.join("link_to_dir");
        symlink_dir(&target_dir, &symlink_path).unwrap();

        let scrubber = link.scrub().path(target.anchor(), "");
        insta::assert_snapshot!(report(&scrubber, &symlink_path), @r"
        exists `/path/to/link/link_to_dir`
         - `/path/to/link/link_to_dir`
                          ^^^^^^^^^^^
                          ↳ Symlink, resolves to dir [✅ read, ✅ write, ✅ execute]
                          ↳ Target `/path/to/target/target_dir`
         - `/path/to/link/link_to_dir`
                     ^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `link_to_dir` (exists)
        🛑
        ");
    }

    // Unix-only test, though the failure it covers is not unix-only. `CannotReadCWD` fires
    // whenever `std::env::current_dir()` fails, which can happen on Windows too (e.g. the working
    // directory lived on a removable or network drive that went away, or its permissions were
    // revoked out from under the process).
    //
    // What differs is how to *provoke* it deterministically. On unix we just delete the CWD while
    // sitting in it. Windows holds an open handle to the CWD and refuses to remove it, so that
    // trick is unavailable, and the remaining triggers cannot be staged reliably from a unit test.
    #[test]
    #[cfg(unix)]
    fn test_cannot_read_cwd() {
        let fixture = Fixture::new();
        fixture.enter();

        let root = fixture.root().to_path_buf();
        std::fs::remove_dir(&root).unwrap();

        let scrubber = Scrubber::new().error(std::fs::read_to_string(&root).unwrap_err());
        insta::assert_snapshot!(report(&scrubber, "relative_path.txt"), @r"
        `relative_path.txt`
         - `relative_path.txt`
            ↳ Cannot read the current directory: {error}
        🛑
        ");
    }

    // Root detection is "the path has no lexical parent". The spelling of a root differs by
    // platform, so each OS asserts its own: unix's `/` here, Windows's `C:\` below. On Windows `/`
    // is root-but-relative (it has a RootDir component but no drive prefix), so it would not
    // report as root.
    #[test]
    #[cfg(unix)]
    fn test_is_root() {
        insta::assert_snapshot!(Scrubber::new().plain(&Report::new("/").to_string()), @r"
        is root `/`
        🛑
        ");
    }

    // A drive root like `C:\` has a Prefix and a RootDir but no further components, so it has no
    // lexical parent and reports as root, the same property `/` has on unix. Unscrubbed: the
    // backslashes and the verbatim prefix are the point, and this snapshot can only ever be
    // recorded by a Windows runner.
    #[test]
    #[cfg(windows)]
    fn test_is_root_windows() {
        insta::assert_snapshot!(format!("{}{STOP}", Report::new(r"C:\")), @r"
        is root `C:\` → `\\?\C:\`
        🛑
        ");
    }

    #[test]
    fn test_prior_dir_problem_relative_path() {
        let fixture = Fixture::new();
        fixture.enter();

        let scrubber = fixture.scrub().not_found("a");
        insta::assert_snapshot!(report(&scrubber, "a/b/c/does_not_exist.txt"), @r"
        does not exist `a/b/c/does_not_exist.txt`
         - `a/b/c/does_not_exist.txt`
            ^
            ↳ Missing: {error}
            ↳ Absolute `/path/to/directory/a/b/c/does_not_exist.txt`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `a`
                     ↳ Contains (0)
                       └── (empty)
        🛑
        ");
    }

    #[test]
    fn test_parent_directory_missing_write_permissions() {
        let fixture = Fixture::new();
        let readonly_dir = fixture.join("readonly_dir");
        std::fs::create_dir(&readonly_dir).unwrap();
        let missing = readonly_dir.join("does_not_exist.txt");

        // Read before the deny-write ACE goes on, so the wording comes from an ordinary refusal.
        let scrubber = fixture.scrub().not_found(&missing);

        set_read_only(&readonly_dir).unwrap();

        let output = report(&scrubber, &missing);

        // Remove the deny-write ACE so the tempdir can be cleaned up.
        #[cfg(windows)]
        restore_write(&readonly_dir).unwrap();

        insta::assert_snapshot!(output, @r"
        does not exist `/path/to/directory/readonly_dir/does_not_exist.txt`
         - `/path/to/directory/readonly_dir/does_not_exist.txt`
                                            ^^^^^^^^^^^^^^^^^^
                                            ↳ Missing: {error}
         - `/path/to/directory/readonly_dir/does_not_exist.txt`
                               ^^^^^^^^^^^^
                               ↳ Dir [✅ read, ❌ write, ✅ execute]
                               ↳ Entries can be listed (read permission)
                               ↳ Cannot create, delete, or rename entries (no write permission)
                               ↳ ❌ Missing `does_not_exist.txt`
                               ↳ Contains (0)
                                 └── (empty)
        🛑
        ");
    }

    #[test]
    fn test_cannot_canonicalize_circular_symlink_absolute() {
        let fixture = Fixture::new();
        let link1 = fixture.join("link1");
        let link2 = fixture.join("link2");

        symlink_file(&link2, &link1).unwrap();
        symlink_file(&link1, &link2).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&link1).unwrap_err());
        insta::assert_snapshot!(report(&scrubber, &link1), @r"
        exists `/path/to/directory/link1`
         - `/path/to/directory/link1`
                               ^^^^^
                               ↳ Symlink, cannot follow: {error}
                               ↳ Target `/path/to/directory/link2`
         - `/path/to/directory/link1`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `link1` (exists)
                       └── `link2`
        🛑
        ");
    }

    #[test]
    fn test_cannot_canonicalize_circular_symlink_relative() {
        let fixture = Fixture::new();
        fixture.enter();

        symlink_file("link2", "link1").unwrap();
        symlink_file("link1", "link2").unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize("link1").unwrap_err());
        insta::assert_snapshot!(report(&scrubber, "link1"), @r"
        exists `link1`
         - `link1`
            ^^^^^
            ↳ Symlink, cannot follow: {error}
            ↳ Target `link2` → `/path/to/directory/link2`
            ↳ Absolute `/path/to/directory/link1`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `link1` (exists)
                       └── `link2`
        🛑
        ");
    }

    #[test]
    fn test_cannot_canonicalize_broken_symlink_absolute() {
        let fixture = Fixture::new();
        let broken_link = fixture.join("broken_link");

        symlink_file(fixture.join("does_not_exist"), &broken_link).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(report(&scrubber, &broken_link), @r"
        exists `/path/to/directory/broken_link`
         - `/path/to/directory/broken_link`
                               ^^^^^^^^^^^
                               ↳ Symlink, cannot follow: {error}
                               ↳ Target `/path/to/directory/does_not_exist`
         - `/path/to/directory/broken_link`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `broken_link` (exists)
        🛑
        ");
    }

    #[test]
    fn test_broken_symlink_prior_absolute() {
        let fixture = Fixture::new();
        let broken_link = fixture.join("broken_link");

        symlink_file(fixture.join("does_not_exist"), &broken_link).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(
            report(&scrubber, broken_link.join("and").join("more.txt")),
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - `/path/to/directory/broken_link/and/more.txt`
                               ^^^^^^^^^^^
                               ↳ Symlink, cannot follow: {error}
                               ↳ Target `/path/to/directory/does_not_exist`
         - `/path/to/directory/broken_link/and/more.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `broken_link` (exists)
        🛑
        "
        );
    }

    #[test]
    fn test_broken_symlink_prior_relative() {
        let fixture = Fixture::new();
        fixture.enter();
        let broken_link = fixture.join("broken_link");

        symlink_file(Path::new("..").join("does_not_exist"), &broken_link).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(
            report(&scrubber, broken_link.join("and").join("more.txt")),
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - `/path/to/directory/broken_link/and/more.txt`
                               ^^^^^^^^^^^
                               ↳ Symlink, cannot follow: {error}
                               ↳ Target `../does_not_exist` → `/path/to/does_not_exist`
         - `/path/to/directory/broken_link/and/more.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `broken_link` (exists)
        🛑
        "
        );
    }

    /// A symlink-to-directory ancestor whose target holds a file that callouts descent. The early
    /// stop is reported at its physical location under the resolved target (`.../real/a.txt`), not
    /// under the link's own name, pinning that `step.at` is physical-through-parent and not a
    /// lexical spelling of the input.
    #[test]
    fn test_prior_path_is_file_under_a_symlinked_directory() {
        let fixture = Fixture::new();
        let real = fixture.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("a.txt"), "").unwrap();
        symlink_dir(&real, fixture.join("linkdir")).unwrap();

        let path = fixture.join("linkdir").join("a.txt").join("b").join("x");

        insta::assert_snapshot!(report(&fixture.scrub(), path), @r"
        does not exist `/path/to/directory/linkdir/a.txt/b/x`
         - `/path/to/directory/linkdir/a.txt/b/x`
                                       ^^^^^
                                       ↳ File, not a dir [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/linkdir/a.txt/b/x`
                               ^^^^^^^
                               ↳ Dir [✅ read, ✅ write, ✅ execute]
                               ↳ Contains (1)
                                 └── `a.txt` (exists)
        🛑
        ");
    }

    #[test]
    fn test_cannot_canonicalize_broken_symlink_relative() {
        let fixture = Fixture::new();
        fixture.enter();

        symlink_file("does_not_exist", "broken_link").unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::canonicalize("broken_link").unwrap_err());
        insta::assert_snapshot!(report(&scrubber, "broken_link"), @r"
        exists `broken_link`
         - `broken_link`
            ^^^^^^^^^^^
            ↳ Symlink, cannot follow: {error}
            ↳ Target `does_not_exist` → `/path/to/directory/does_not_exist`
            ↳ Absolute `/path/to/directory/broken_link`
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `broken_link` (exists)
        🛑
        ");
    }

    // Unix-only: the behavior is not reproducible on the Windows CI runner.
    //
    // The test needs "a directory you can list but not descend into" so that the lstat on a child
    // is refused. Unix models this as the directory execute bit, dropped here with
    // `set_mode(0o644)`. Windows models it as the "Traverse folder / execute file" ACL right, and
    // GitHub's Windows runner executes as an administrator whose token bypasses per-user traverse
    // denials, so `icacls /deny` leaves the lstat succeeding.
    #[test]
    #[cfg(unix)]
    fn test_no_execute_dir_with_file() {
        let fixture = Fixture::new();
        let no_exec_dir = fixture.join("no_exec_dir");
        std::fs::create_dir(&no_exec_dir).unwrap();

        let file = no_exec_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        set_mode(&no_exec_dir, 0o644).unwrap(); // read + write, no execute

        let scrubber = fixture
            .scrub()
            .error(std::fs::symlink_metadata(&file).unwrap_err());
        let output = report(&scrubber, &file);

        set_mode(&no_exec_dir, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/no_exec_dir/file.txt`
         - `/path/to/directory/no_exec_dir/file.txt`
                                           ^^^^^^^^
                                           ↳ Parent directory missing execute permission
                                           ↳ Cannot lstat: {error}
         - `/path/to/directory/no_exec_dir/file.txt`
                               ^^^^^^^^^^^
                               ↳ Dir [✅ read, ✅ write, ❌ execute]
                               ↳ Entries can be listed (read permission)
                               ↳ Cannot enter, traverse, or access entries (no execute permission)
                               ↳ Contains (1)
                                 └── `file.txt` (exists)
        🛑
        ");
    }

    // Unix-only for the same reason as the test above: it removes directory traverse (0o444) so
    // the lstat on a child is refused, which the Windows runner's admin token bypasses.
    #[test]
    #[cfg(unix)]
    fn test_no_write_dir_with_file() {
        let fixture = Fixture::new();
        let no_write_dir = fixture.join("no_write_dir");
        std::fs::create_dir(&no_write_dir).unwrap();

        let file = no_write_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        // read only: no write (fires the warning), no execute (the lstat is refused)
        set_mode(&no_write_dir, 0o444).unwrap();

        let scrubber = fixture
            .scrub()
            .error(std::fs::symlink_metadata(&file).unwrap_err());
        let output = report(&scrubber, &file);

        set_mode(&no_write_dir, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/no_write_dir/file.txt`
         - `/path/to/directory/no_write_dir/file.txt`
                                            ^^^^^^^^
                                            ↳ Parent directory missing execute permission
                                            ↳ Cannot lstat: {error}
         - `/path/to/directory/no_write_dir/file.txt`
                               ^^^^^^^^^^^^
                               ↳ Dir [✅ read, ❌ write, ❌ execute]
                               ↳ Entries can be listed (read permission)
                               ↳ Cannot create, delete, or rename entries (no write permission)
                               ↳ Cannot enter, traverse, or access entries (no execute permission)
                               ↳ Contains (1)
                                 └── `file.txt` (exists)
        🛑
        ");
    }

    // Unix-only: builds `AbsPath` from unix-rooted paths (`/` and `/pretend/...`), which are not
    // valid absolute paths on Windows (roots look like `C:\`).
    #[test]
    #[cfg(unix)]
    fn test_cannot_canonicalize_anything() {
        use crate::abs_path::AbsPath;

        let path = PathBuf::from("/pretend/root/does/not/exist/somehow");
        let report = Report {
            path: path.clone(),
            trace: Err(CannotTrace::RootNotReachable(CannotCanonicalizeAnything {
                original: AbsPath::new(&path).unwrap(),
                root: AbsPath::new(Path::new("/")).unwrap(),
                root_error: std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "simulated error",
                ),
            })),
        };

        insta::assert_snapshot!(Scrubber::new().carets(&report.to_string()), @r"
        `/pretend/root/does/not/exist/somehow`
         - `/pretend/root/does/not/exist/somehow`
                                         ^^^^^^^
                                         ↳ Root `/` is not reachable
                                         ↳ Cannot canonicalize: simulated error
        🛑
        ");
    }

    /// A race caught on the final component. The walk resolved every step, then a re-check
    /// contradicted what it had just seen. A real race cannot be timed on demand, so
    /// `Trace::append_race` writes a known one into an otherwise real trace.
    #[test]
    fn test_renders_a_raced_final_component() {
        let fixture = Fixture::new();
        let path = fixture.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        let mut trace = Trace::new(&path).unwrap();
        trace.append_race(
            "realpath resolved this link, stat failed to resolve",
            std::fs::metadata(fixture.join("does_not_exist")).unwrap_err(),
        );

        let report = Report::from_trace_result(path.clone(), Ok(trace));

        let scrubber = fixture.scrub().not_found(fixture.join("does_not_exist"));
        insta::assert_snapshot!(scrubber.carets(&report.to_string()), @r"
        `/path/to/directory/a/b`
         - `/path/to/directory/a/b`
                                 ^
                                 ↳ ⚠️ Filesystem change detected while gathering facts.
                                   ⚠️ Facts displayed may be invalid, stale or disagree.
                                   ⚠️
                                   ⚠️ Detected: realpath resolved this link, stat failed to resolve
                                   ⚠️ Error: {error}
         - `/path/to/directory/a/b`
                               ^
                               ↳ Dir [✅ read, ✅ write, ✅ execute]
                               ↳ Contains (1)
                                 └── `b` (exists)
        🛑
        ");
    }
}

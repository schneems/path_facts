//! The callout rendering of a path walk.
//!
//! [`Report`] holds the [`Display`] information for [`PathFacts`](crate::PathFacts).

use crate::abs_path::AbsPathError;
use crate::callout::{self, Callout, Caret, Entry, Fact, Row};
use crate::canonical_path::{CannotCanonicalizeAnything, CanonicalPath};
use crate::env_config;
use crate::non_empty::NonEmpty;
use crate::style;
use crate::trace::{CannotTrace, PhysicalNode, ResolvedSymlink, StatusOnDisk, Step, Trace};
use faccess::{AccessMode, PathExt};
use std::fmt::Display;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

/// Shows facts about a path as a series of caret-annotated callouts when `Display`ed.
#[derive(Debug)]
pub(crate) struct Report {
    /// Original input path, exactly as the caller spelled it
    path: PathBuf,
    /// Detected state of the path, paired with the callouts built from it
    trace: Result<(Trace, NonEmpty<Callout>), CannotTrace>,
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
        Self::with_dir_limit(path, result, env_config::get_config().dir_limit)
    }

    fn with_dir_limit(
        path: impl AsRef<Path>,
        result: Result<Trace, CannotTrace>,
        dir_limit: usize,
    ) -> Self {
        let trace = result.map(|trace| trace_and_callouts(trace, dir_limit));
        Report {
            path: path.as_ref().to_path_buf(),
            trace,
        }
    }

    pub(crate) fn resolved_location(&self) -> Option<CanonicalPath> {
        let (trace, _) = self.trace.as_ref().ok()?;
        trace.physical_location()
    }

    pub(crate) fn annotate_stopped(&mut self, note: String) {
        if let Ok((_, callouts)) = &mut self.trace {
            callouts.first_mut().facts.push(Fact::Text(note));
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
            self.existence(trace),
            callout::render_callouts(callouts)
        )
    }

    pub(crate) fn render_with_prefix(&self, prefix: &str) -> String {
        let (trace, callouts) = match self.trace.as_ref() {
            Ok(walked) => walked,
            Err(cannot) => return format!("{prefix}{}", self.render_cannot_trace(cannot)),
        };

        let summary = self.existence(trace);
        let path_col = prefix.rsplit('\n').next().unwrap_or("").chars().count()
            + summary
                .chars()
                .position(|char| char == '`')
                .map_or(0, |index| index + 1);

        let (first, rest) = callouts.split_first();
        let mut out = format!(
            "{prefix}{summary}\n{}",
            callout::render_caret_and_facts(first, path_col)
        );
        if !rest.is_empty() {
            out.push('\n');
            out.push_str(&callout::render_callouts(rest));
        }
        out
    }

    fn existence(&self, trace: &Trace) -> String {
        let input = self.path.display();
        match trace.status_on_disk() {
            StatusOnDisk::Exists => format!("exists `{input}`"),
            StatusOnDisk::DoesNotExist => format!("does not exist `{input}`"),
            StatusOnDisk::Unknown => format!("`{input}`"),
        }
    }

    /// What to print when we couldn't start a [`Trace`]
    fn render_cannot_trace(&self, cannot: &CannotTrace) -> String {
        let input = self.path.display();
        match cannot {
            CannotTrace::CannotAbsolute(AbsPathError::NulChar(path)) => {
                format!(
                    "path `{}` contains a NUL character {:?}",
                    path.display(),
                    path.display()
                )
            }
            CannotTrace::CannotAbsolute(AbsPathError::PathIsEmpty(path)) => {
                format!("path `{}` is empty", path.display())
            }
            CannotTrace::CannotAbsolute(AbsPathError::CannotReadCWD(path, error)) => {
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

fn trace_and_callouts(trace: Trace, dir_limit: usize) -> (Trace, NonEmpty<Callout>) {
    let callouts = callouts(&trace, dir_limit);
    (trace, callouts)
}

/// Build Callouts for a trace
fn callouts(trace: &Trace, dir_limit: usize) -> NonEmpty<Callout> {
    let status = trace.stop_status();
    let mut callouts = NonEmpty::new(stopped_step_callout(
        trace,
        status.step(),
        status.stopped_early(),
    ));
    if let Some(parent) = parent_of_stopped_callout(trace, dir_limit) {
        callouts.push(parent);
    }
    callouts
}

/// The caller's whole path, caret on the component the walk stopped at.
fn stopped_step_callout(trace: &Trace, step: &Step, early: bool) -> Callout {
    let path = trace.input();
    let mut facts = vec![Fact::Text(stopped_step_type_line(step, early))];
    facts.extend(stopped_step_detail_facts(step));

    // We may need to show the full path later, ensure the end user can see at a glance
    // how the full path is connected to the original input
    if path.is_relative() {
        facts.push(Fact::Text(format!("Absolute {}", trace.absolute())));
    }

    if let Some(canonical) = resolved_elsewhere(step) {
        facts.push(Fact::Text(format!("Canonical {canonical}")));
    }

    Callout {
        path: path.display().to_string(),
        caret: callout::highlight(path, step.input_index.map_or(Caret::Last, Caret::Index)),
        facts,
    }
}

/// Show the directory with contents holding the stopped step.
fn parent_of_stopped_callout(trace: &Trace, dir_limit: usize) -> Option<Callout> {
    let path = trace.input();
    let listing = trace.listing()?;
    let dir = &listing.dir_before;
    let sought = &listing.component;

    let (shown, caret) = if let Some(span) = trace
        .input_index_of_dir(&listing)
        .and_then(|index| callout::highlight(path, Caret::Index(index)))
    {
        (path.display().to_string(), Some(span))
    } else {
        // When the dir_before + component cannot be mapped to an input index show the full path of the
        // directory + the component.
        let span = callout::highlight(dir.as_ref(), Caret::Last);
        (
            dir.as_ref().join(sought.as_ref()).display().to_string(),
            span,
        )
    };

    let mut facts = vec![Fact::Text(format!("Dir {}", triad(dir.as_ref())))];
    facts.extend(dir_permission_facts(dir.as_ref()));

    let entries = dir.normal_entries();
    match &entries {
        Err(error) => facts.push(Fact::Text(format!("Cannot list directory: {error}"))),
        Ok(entries) if !entries.contains(sought) => {
            facts.push(Fact::Text(format!("❌ Missing {sought}")));
        }
        Ok(_) => {}
    }

    // List directory contents last so it doesn't come between two facts
    if let Ok(entries) = &entries {
        let mut rows = Vec::new();
        let mut others = 0;
        let mut hidden = 0;
        for entry in entries {
            let is_sought = entry == sought;
            if !is_sought {
                others += 1;
                if others > dir_limit {
                    hidden += 1;
                    continue;
                }
            }

            if let Some(value) = NonZeroUsize::new(hidden) {
                rows.push(Row::Hidden(value));
                hidden = 0;
            }
            rows.push(Row::Entry(Entry {
                name: entry.clone(),
                annotation: is_sought.then(|| "(exists)".to_string()),
            }));
        }
        if let Some(value) = NonZeroUsize::new(hidden) {
            rows.push(Row::Hidden(value));
        }
        facts.push(Fact::Contains { rows });
    }

    Some(Callout {
        path: shown,
        caret,
        facts,
    })
}

/// First description of the step that stopped (should hint why it stopped there when early)
fn stopped_step_type_line(step: &Step, early: bool) -> String {
    let not_a_dir = if early { ", not a dir" } else { "" };

    match &step.contents {
        PhysicalNode::Directory(canonical) => format!("Dir {}", triad(canonical.as_ref())),
        PhysicalNode::File(canonical) => {
            format!("File{not_a_dir} {}", triad(canonical.as_ref()))
        }
        PhysicalNode::ParentDir { resolved, .. } => format!("Dir {}", triad(resolved.as_ref())),
        PhysicalNode::Symlink {
            resolved: Ok(ResolvedSymlink::Dir(canonical)),
            ..
        } => format!("Symlink, resolves to dir {}", triad(canonical.as_ref())),
        PhysicalNode::Symlink {
            resolved: Ok(ResolvedSymlink::File(canonical)),
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
        PhysicalNode::UnknownLookup(error) => format!("Cannot read raw metadata: {error}"),
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
            let message = "⚠️ Unexpected stopped_step_type_line: PhysicalNode::NotReached";

            if cfg!(test) {
                unreachable!("{}", message);
            }
            String::from(message)
        }
    }
}

/// Extra details about a stopped step beyond [`stopped_step_type_line`]
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
            Err(error) => vec![Fact::Text(format!("Cannot read symlink target: {error}"))],
        },
        PhysicalNode::ParentNoExec { error, .. } => {
            vec![Fact::Text(format!("Cannot read raw metadata: {error}"))]
        }
        _ => Vec::new(),
    }
}

/// Physical resolved path if it's different than what's already visible in callout
/// Helps avoid repetition of facts already visible on the screen
fn resolved_elsewhere(step: &Step) -> Option<CanonicalPath> {
    match &step.contents {
        // The `..` in `/path/to/directory/a/b/..` is always shown as `/path/to/directory/a`
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

/// Directory permission capabilities spelled out in words
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_unfolded;
    use crate::test_support::*;

    fn report(scrubber: &Scrubber, path: impl AsRef<Path>) -> String {
        scrubber.carets(&Report::new(path).to_string())
    }

    fn report_with_dir_limit(scrubber: &Scrubber, path: &Path, dir_limit: usize) -> String {
        scrubber.carets(&Report::with_dir_limit(path, Trace::new(path), dir_limit).to_string())
    }

    #[test]
    fn test_dir_limit_keeps_sought_entry_past_the_cutoff() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"] {
            std::fs::write(fixture.join(name), "").unwrap();
        }

        let output = report_with_dir_limit(&fixture.scrub(), &fixture.join("e.txt"), 2);

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/e.txt`
         - `/path/to/directory/e.txt`
                               ^^^^^
                               > File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/e.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (5)
                       ├── `a.txt`
                       ├── `b.txt`
                       ├── ...
                       └── `e.txt` (exists)
        🛑
        ");
    }

    #[test]
    fn test_dir_limit_zero_with_sought_entry_first() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(fixture.join(name), "").unwrap();
        }

        let output = report_with_dir_limit(&fixture.scrub(), &fixture.join("a.txt"), 0);

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/a.txt`
         - `/path/to/directory/a.txt`
                               ^^^^^
                               > File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/a.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (3)
                       ├── `a.txt` (exists)
                       └── ...
        🛑
        ");
    }

    #[test]
    fn test_dir_limit_zero_with_sought_entry_last() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(fixture.join(name), "").unwrap();
        }

        let output = report_with_dir_limit(&fixture.scrub(), &fixture.join("c.txt"), 0);

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/c.txt`
         - `/path/to/directory/c.txt`
                               ^^^^^
                               > File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/c.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (3)
                       ├── ...
                       └── `c.txt` (exists)
        🛑
        ");
    }

    #[test]
    fn test_dir_limit_cuts_both_sides_of_sought_entry() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt"] {
            std::fs::write(fixture.join(name), "").unwrap();
        }

        let output = report_with_dir_limit(&fixture.scrub(), &fixture.join("d.txt"), 2);

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/d.txt`
         - `/path/to/directory/d.txt`
                               ^^^^^
                               > File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/d.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (6)
                       ├── `a.txt`
                       ├── `b.txt`
                       ├── ...
                       ├── `d.txt` (exists)
                       └── ...
        🛑
        ");
    }

    #[test]
    fn test_dir_limit_zero_with_missing_entry() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(fixture.join(name), "").unwrap();
        }

        let missing = fixture.join("missing.txt");
        let scrubber = fixture.scrub().not_found(&missing);
        let output = report_with_dir_limit(&scrubber, &missing, 0);

        insta::assert_snapshot!(output, @r"
        does not exist `/path/to/directory/missing.txt`
         - `/path/to/directory/missing.txt`
                               ^^^^^^^^^^^
                               > Missing: {error}
         - `/path/to/directory/missing.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > ❌ Missing `missing.txt`
                     > Contains (3)
                       └── ...
        🛑
        ");
    }

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
                                           > File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/search_only/child.txt`
                               ^^^^^^^^^^^
                               > Dir [❌ read, ❌ write, ✅ execute]
                               > Cannot list entries (no read permission)
                               > Cannot create, delete, or rename entries (no write permission)
                               > Cannot list directory: Permission denied (os error 13)
        🛑
        ");
    }

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

        set_mode(&denied, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        `/path/to/directory/denied/child.txt`
         - `/path/to/directory/denied/child.txt`
                                      ^^^^^^^^^
                                      > Cannot read raw metadata: Permission denied (os error 13)
         - `/path/to/directory/denied/child.txt`
                               ^^^^^^
                               > Dir [❌ read, ❌ write, ❌ execute]
                               > Cannot list entries (no read permission)
                               > Cannot create, delete, or rename entries (no write permission)
                               > Cannot enter, traverse, or access entries (no execute permission)
                               > Cannot list directory: Permission denied (os error 13)
        🛑
        ");
    }

    #[test]
    fn render_with_prefix_empty_folds_onto_summary() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();
        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::assert_snapshot!(
            fixture.scrub().carets(&Report::new(&path).render_with_prefix("")),
            @r"
        does not exist `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                                           ^^^^^
                                           > File, not a dir [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `a.txt` (exists)
        🛑
        "
        );
    }

    #[test]
    fn render_with_prefix_label_accounts_for_its_width() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();
        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::assert_snapshot!(
            fixture.scrub().carets(&Report::new(&path).render_with_prefix("Path ")),
            @r"
        Path does not exist `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                                                ^^^^^
                                                > File, not a dir [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `a.txt` (exists)
        🛑
        "
        );
    }

    #[test]
    fn render_with_prefix_measures_only_the_last_line() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();
        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::assert_snapshot!(
            fixture.scrub().carets(&Report::new(&path).render_with_prefix("lead\n")),
            @r"
        lead
        does not exist `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                                           ^^^^^
                                           > File, not a dir [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `a.txt` (exists)
        🛑
        "
        );
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
                               > Missing: {error}
         - `/path/to/directory/a/b/c/does_not_exist.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > ❌ Missing `a`
                     > Contains (0)
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
                               > File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/exists.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `exists.txt` (exists)
        🛑
        ");
    }

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
                .map(std::path::Component::as_os_str)
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
                let trace = Trace::new(path).unwrap();
                let answer = match resolved_elsewhere(trace.stop_status().step()) {
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

    /// `<dir>/a/b/..` resolves to `<dir>/a`, so the directory to list is `<dir>`.
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
                                   > Dir [✅ read, ✅ write, ✅ execute]
                                   > Canonical `/path/to/directory/a`
         - `/path/to/directory/a/b/..`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (2)
                       ├── `a` (exists)
                       └── `other.txt`
        🛑
        ");
    }

    #[test]
    fn test_exists_repeated_dot_dot_annotates_resolved_entry() {
        let fixture = Fixture::new();
        let a = fixture.join("a");
        std::fs::create_dir_all(a.join("b")).unwrap();
        std::fs::write(fixture.join("other.txt"), "").unwrap();

        insta::assert_snapshot!(report(&fixture.scrub(), join_unfolded(&a, &["..", "a", "b", ".."])), @r"
        exists `/path/to/directory/a/../a/b/..`
         - `/path/to/directory/a/../a/b/..`
                                        ^^
                                        > Dir [✅ read, ✅ write, ✅ execute]
                                        > Canonical `/path/to/directory/a`
         - `/path/to/directory/a/../a/b/..`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (2)
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
                               > Missing: {error}
         - `/path/to/directory/does_not_exist.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > ❌ Missing `does_not_exist.txt`
                     > Contains (0)
                       └── (empty)
        🛑
        ");
    }

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
            > Missing: {error}
            > Absolute `/path/to/directory/doesnotexist.txt`
         - `/path/to/directory/doesnotexist.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > ❌ Missing `doesnotexist.txt`
                     > Contains (0)
                       └── (empty)

        To path does not exist `also_does_not_exist.txt`
         - `also_does_not_exist.txt`
            ^^^^^^^^^^^^^^^^^^^^^^^
            > Missing: {error}
            > Absolute `/path/to/directory/also_does_not_exist.txt`
         - `/path/to/directory/also_does_not_exist.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > ❌ Missing `also_does_not_exist.txt`
                     > Contains (0)
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
            > File [✅ read, ✅ write, ❌ execute]
            > Absolute `/path/to/directory/exists.txt`
         - `/path/to/directory/exists.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
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
                          > Symlink, resolves to file [✅ read, ✅ write, ❌ execute]
                          > Target `/path/to/target/target.txt`
         - `/path/to/link/link_to_target.txt`
                     ^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `link_to_target.txt` (exists)
        🛑
        ");
    }

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
                          > Symlink, resolves to file, not a dir [✅ read, ✅ write, ❌ execute]
                          > Target `/path/to/target/target.txt`
         - `/path/to/link/link_to_target.txt/below.txt`
                     ^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
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
                          > Symlink, resolves to dir [✅ read, ✅ write, ✅ execute]
                          > Target `/path/to/target/target_dir`
         - `/path/to/link/link_to_dir`
                     ^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `link_to_dir` (exists)
        🛑
        ");
    }

    #[test]
    #[cfg(unix)]
    fn test_cannot_read_cwd() {
        let fixture = Fixture::new();
        fixture.enter();

        let root = fixture.root().to_path_buf();
        std::fs::remove_dir(&root).unwrap();

        let scrubber = Scrubber::new().error(&std::fs::read_to_string(&root).unwrap_err());
        insta::assert_snapshot!(report(&scrubber, "relative_path.txt"), @r"
        `relative_path.txt`
         - `relative_path.txt`
            > Cannot read the current directory: {error}
        🛑
        ");
    }

    #[test]
    fn test_nul_char() {
        let output = Report::new("hello\0world").to_string();
        insta::assert_snapshot!(output, @"path `hello\0world` contains a NUL character \"hello\\0world\"");
    }

    #[test]
    #[cfg(unix)]
    fn test_is_root() {
        insta::assert_snapshot!(Scrubber::new().plain(&Report::new("/").to_string()), @r"
        is root `/`
        🛑
        ");
    }

    #[test]
    #[cfg(windows)]
    fn test_is_root_windows() {
        insta::assert_snapshot!(format!("{}{STOP}", Report::new(r"C:\")), @r"
        is root `C:\` → `\\?\C:\`
        🛑
        ");
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_relative_parent_dir_is_folded_by_absolute() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.join("a").join("b")).unwrap();
        fixture.enter();

        let input = r"a\b\..";
        let absolute = std::path::absolute(input).unwrap();
        assert!(
            !absolute
                .components()
                .any(|c| c == std::path::Component::ParentDir),
            "expected `absolute` to fold `..`, got {}",
            absolute.display()
        );

        insta::assert_snapshot!(report(&fixture.scrub(), input), @r"
        exists `a/b/..`
         - `a/b/..`
                ^^
                > Dir [✅ read, ✅ write, ✅ execute]
                > Absolute `/path/to/directory/a`
         - `/path/to/directory/a`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `a` (exists)
        🛑
        ");
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_relative_trailing_dot_is_stripped_by_absolute() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.join("a").join("b")).unwrap();
        std::fs::write(fixture.join("a").join("b").join("c"), "").unwrap();
        fixture.enter();

        let input = r"a\b\c.";
        let absolute = std::path::absolute(input).unwrap();
        assert_eq!(
            absolute.file_name(),
            Some(std::ffi::OsStr::new("c")),
            "expected `absolute` to strip the trailing `.`, got {}",
            absolute.display()
        );

        insta::assert_snapshot!(report(&fixture.scrub(), input), @r"
        exists `a/b/c.`
         - `a/b/c.`
                ^^
                > File [✅ read, ✅ write, ✅ execute]
                > Absolute `/path/to/directory/a/b/c`
         - `/path/to/directory/a/b/c`
                                 ^
                                 > Dir [✅ read, ✅ write, ✅ execute]
                                 > Contains (1)
                                   └── `c` (exists)
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
            > Missing: {error}
            > Absolute `/path/to/directory/a/b/c/does_not_exist.txt`
         - `/path/to/directory/a`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > ❌ Missing `a`
                     > Contains (0)
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
                                            > Missing: {error}
         - `/path/to/directory/readonly_dir/does_not_exist.txt`
                               ^^^^^^^^^^^^
                               > Dir [✅ read, ❌ write, ✅ execute]
                               > Entries can be listed (read permission)
                               > Cannot create, delete, or rename entries (no write permission)
                               > ❌ Missing `does_not_exist.txt`
                               > Contains (0)
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
            .error(&std::fs::canonicalize(&link1).unwrap_err());
        insta::assert_snapshot!(report(&scrubber, &link1), @r"
        exists `/path/to/directory/link1`
         - `/path/to/directory/link1`
                               ^^^^^
                               > Symlink, cannot follow: {error}
                               > Target `/path/to/directory/link2`
         - `/path/to/directory/link1`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (2)
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
            .error(&std::fs::canonicalize("link1").unwrap_err());
        insta::assert_snapshot!(report(&scrubber, "link1"), @r"
        exists `link1`
         - `link1`
            ^^^^^
            > Symlink, cannot follow: {error}
            > Target `link2` → `/path/to/directory/link2`
            > Absolute `/path/to/directory/link1`
         - `/path/to/directory/link1`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (2)
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
            .error(&std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(report(&scrubber, &broken_link), @r"
        exists `/path/to/directory/broken_link`
         - `/path/to/directory/broken_link`
                               ^^^^^^^^^^^
                               > Symlink, cannot follow: {error}
                               > Target `/path/to/directory/does_not_exist`
         - `/path/to/directory/broken_link`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
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
            .error(&std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(
            report(&scrubber, broken_link.join("and").join("more.txt")),
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - `/path/to/directory/broken_link/and/more.txt`
                               ^^^^^^^^^^^
                               > Symlink, cannot follow: {error}
                               > Target `/path/to/directory/does_not_exist`
         - `/path/to/directory/broken_link/and/more.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
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
            .error(&std::fs::canonicalize(&broken_link).unwrap_err());
        insta::assert_snapshot!(
            report(&scrubber, broken_link.join("and").join("more.txt")),
            @r"
        `/path/to/directory/broken_link/and/more.txt`
         - `/path/to/directory/broken_link/and/more.txt`
                               ^^^^^^^^^^^
                               > Symlink, cannot follow: {error}
                               > Target `../does_not_exist` → `/path/to/does_not_exist`
         - `/path/to/directory/broken_link/and/more.txt`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `broken_link` (exists)
        🛑
        "
        );
    }

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
                                       > File, not a dir [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/linkdir/a.txt/b/x`
                               ^^^^^^^
                               > Dir [✅ read, ✅ write, ✅ execute]
                               > Contains (1)
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
            .error(&std::fs::canonicalize("broken_link").unwrap_err());
        insta::assert_snapshot!(report(&scrubber, "broken_link"), @r"
        exists `broken_link`
         - `broken_link`
            ^^^^^^^^^^^
            > Symlink, cannot follow: {error}
            > Target `does_not_exist` → `/path/to/directory/does_not_exist`
            > Absolute `/path/to/directory/broken_link`
         - `/path/to/directory/broken_link`
                     ^^^^^^^^^
                     > Dir [✅ read, ✅ write, ✅ execute]
                     > Contains (1)
                       └── `broken_link` (exists)
        🛑
        ");
    }

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
            .error(&std::fs::symlink_metadata(&file).unwrap_err());
        let output = report(&scrubber, &file);

        set_mode(&no_exec_dir, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/no_exec_dir/file.txt`
         - `/path/to/directory/no_exec_dir/file.txt`
                                           ^^^^^^^^
                                           > Parent directory missing execute permission
                                           > Cannot read raw metadata: {error}
         - `/path/to/directory/no_exec_dir/file.txt`
                               ^^^^^^^^^^^
                               > Dir [✅ read, ✅ write, ❌ execute]
                               > Entries can be listed (read permission)
                               > Cannot enter, traverse, or access entries (no execute permission)
                               > Contains (1)
                                 └── `file.txt` (exists)
        🛑
        ");
    }

    #[test]
    #[cfg(unix)]
    fn test_no_write_dir_with_file() {
        let fixture = Fixture::new();
        let no_write_dir = fixture.join("no_write_dir");
        std::fs::create_dir(&no_write_dir).unwrap();

        let file = no_write_dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();

        // read only: no write (fires the warning), no execute (`symlink_metadata` is refused)
        set_mode(&no_write_dir, 0o444).unwrap();

        let scrubber = fixture
            .scrub()
            .error(&std::fs::symlink_metadata(&file).unwrap_err());
        let output = report(&scrubber, &file);

        set_mode(&no_write_dir, 0o755).unwrap();

        insta::assert_snapshot!(output, @r"
        exists `/path/to/directory/no_write_dir/file.txt`
         - `/path/to/directory/no_write_dir/file.txt`
                                            ^^^^^^^^
                                            > Parent directory missing execute permission
                                            > Cannot read raw metadata: {error}
         - `/path/to/directory/no_write_dir/file.txt`
                               ^^^^^^^^^^^^
                               > Dir [✅ read, ❌ write, ❌ execute]
                               > Entries can be listed (read permission)
                               > Cannot create, delete, or rename entries (no write permission)
                               > Cannot enter, traverse, or access entries (no execute permission)
                               > Contains (1)
                                 └── `file.txt` (exists)
        🛑
        ");
    }

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
                                         > Root `/` is not reachable
                                         > Cannot canonicalize: simulated error
        🛑
        ");
    }

    #[test]
    fn test_renders_a_raced_final_component() {
        let fixture = Fixture::new();
        let path = fixture.join("a").join("b");
        std::fs::create_dir_all(&path).unwrap();

        let mut trace = Trace::new(&path).unwrap();
        trace.append_race(
            "the link canonicalized, but reading its resolved metadata failed",
            std::fs::metadata(fixture.join("does_not_exist")).unwrap_err(),
        );

        let report = Report::from_trace_result(path.clone(), Ok(trace));

        let scrubber = fixture.scrub().not_found(fixture.join("does_not_exist"));
        insta::assert_snapshot!(scrubber.carets(&report.to_string()), @r"
        `/path/to/directory/a/b`
         - `/path/to/directory/a/b`
                                 ^
                                 > ⚠️ Filesystem change detected while gathering facts.
                                   ⚠️ Facts displayed may be invalid, stale or disagree.
                                   ⚠️
                                   ⚠️ Detected: the link canonicalized, but reading its resolved metadata failed
                                   ⚠️ Error: {error}
         - `/path/to/directory/a/b`
                               ^
                               > Dir [✅ read, ✅ write, ✅ execute]
                               > Contains (1)
                                 └── `b` (exists)
        🛑
        ");
    }
}

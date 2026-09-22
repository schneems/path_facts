use crate::report::Report;
use std::{fmt::Display, path::Path};

/// Shows helpful facts about two paths when `Display`ed.
///
/// The output already contains "leader" text, i.e. `From path exists`, which a plain
/// [`PathFacts`](crate::PathFacts) does not. Each half is otherwise the same information two
/// separate [`PathFacts`](crate::PathFacts) would give, plus one cross-reference: when both paths
/// resolve to the same location on disk, the `from` half says so and points down at the `to` half
/// below it. That note is the first step toward de-duplicating the information the halves share.
///
/// Display must begin a line, either at the start of the string or immediately after a newline.
/// Each half draws a caret (`^^^^^^^`) and its facts on the lines below, indented to sit under the
/// path in the summary above. Text printed in front of the first line does not shift those lower
/// lines, so it would knock the caret out from under the path.
#[derive(Debug)]
pub struct FromTo {
    from: Report,
    to: Report,
}

impl FromTo {
    pub fn new(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Self {
        let mut from = Report::new(from.as_ref());
        let to = Report::new(to.as_ref());

        if same_location(&from, &to) {
            from.annotate_leaf("Same location as to path (below)".to_string());
        }

        FromTo { from, to }
    }
}

/// Whether both paths resolve to a single shared entry on disk.
///
/// Compares physical locations, so it holds across a symlink and its target, a folded `..`, and a
/// relative path spelled against an absolute one. `false` whenever either path has no one resolved
/// location — a missing name, a broken link, a root that does not answer — because there is then
/// nothing to equate.
fn same_location(from: &Report, to: &Report) -> bool {
    match (from.resolved_location(), to.resolved_location()) {
        (Some(from), Some(to)) => from == to,
        _ => false,
    }
}

impl Display for FromTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "{}",
            self.from
                .render_with_prefix("From path ")
                .trim_end_matches('\n')
        )?;
        writeln!(
            f,
            "{}",
            self.to
                .render_with_prefix("To path ")
                .trim_end_matches('\n')
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{symlink_file, Fixture};

    /// Two spellings of one file: the `from` symlink `latest.log` resolves to the `to` path
    /// `2024-01.log`, its own target. Because both resolve to one entry on disk, the `from` leaf
    /// gains a `Same location as to path (below)` note pointing down at the `to` half.
    #[test]
    fn from_to_same_file_via_symlink() {
        let fixture = Fixture::new();
        let real = fixture.join("2024-01.log");
        std::fs::write(&real, "").unwrap();
        let link = fixture.join("latest.log");
        symlink_file("2024-01.log", &link).unwrap();

        insta::assert_snapshot!(fixture.scrub().carets(&FromTo::new(&link, &real).to_string()), @r"
        From path exists `/path/to/directory/latest.log`
                                             ^^^^^^^^^^
                                             ↳ Symlink, resolves to file [✅ read, ✅ write, ❌ execute]
                                             ↳ Target `2024-01.log` → `/path/to/directory/2024-01.log`
                                             ↳ Same location as to path (below)
         - `/path/to/directory/latest.log`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `2024-01.log`
                       └── `latest.log` (exists)
        To path exists `/path/to/directory/2024-01.log`
                                           ^^^^^^^^^^^
                                           ↳ File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/2024-01.log`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `2024-01.log` (exists)
                       └── `latest.log`
        🛑
        ");
    }

    #[test]
    fn from_to_same_file_relative_and_absolute() {
        let fixture = Fixture::new();
        fixture.enter();
        std::fs::write(fixture.join("2024-01.log"), "").unwrap();

        let from = Path::new("2024-01.log");
        let to = fixture.join("2024-01.log");

        insta::assert_snapshot!(fixture.scrub().carets(&FromTo::new(from, &to).to_string()), @r"
        From path exists `2024-01.log`
                          ^^^^^^^^^^^
                          ↳ File [✅ read, ✅ write, ❌ execute]
                          ↳ Absolute `/path/to/directory/2024-01.log`
                          ↳ Same location as to path (below)
         - `/path/to/directory/2024-01.log`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `2024-01.log` (exists)
        To path exists `/path/to/directory/2024-01.log`
                                           ^^^^^^^^^^^
                                           ↳ File [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/2024-01.log`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `2024-01.log` (exists)
        🛑
        ");
    }

    #[test]
    fn from_to_two_different_files_are_not_cross_referenced() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("from.txt"), "").unwrap();
        std::fs::write(fixture.join("to.txt"), "").unwrap();

        let output = FromTo::new(fixture.join("from.txt"), fixture.join("to.txt")).to_string();

        assert!(
            output.contains("From path exists") && output.contains("To path exists"),
            "expected both halves to render:\n{}",
            output
        );
        assert!(
            !output.contains("Same location"),
            "two different files must not be cross-referenced:\n{}",
            output
        );
    }

    #[test]
    fn from_to_basic() {
        use indoc::formatdoc;

        let fixture = Fixture::new();
        fixture.enter();

        let from = Path::new("doesnotexist.txt");
        let to = Path::new("also_does_not_exist.txt");

        let error = std::fs::rename(from, to)
            .map_err(|_error| {
                formatdoc! {"
                    cannot rename from `{}` to `{}` due to: {{error}}.

                    {}
                    ",
                    from.display(),
                    to.display(),
                    FromTo::new(from, to)
                }
            })
            .unwrap_err();

        let scrubber = fixture.scrub().not_found(from);
        insta::assert_snapshot!(scrubber.carets(&error), @r"
        cannot rename from `doesnotexist.txt` to `also_does_not_exist.txt` due to: {error}.

        From path does not exist `doesnotexist.txt`
                                  ^^^^^^^^^^^^^^^^
                                  ↳ Missing: {error}
                                  ↳ Absolute `/path/to/directory/doesnotexist.txt`
         - `/path/to/directory/doesnotexist.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `doesnotexist.txt`
                     ↳ Contains (0)
                       └── (empty)
        To path does not exist `also_does_not_exist.txt`
                                ^^^^^^^^^^^^^^^^^^^^^^^
                                ↳ Missing: {error}
                                ↳ Absolute `/path/to/directory/also_does_not_exist.txt`
         - `/path/to/directory/also_does_not_exist.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `also_does_not_exist.txt`
                     ↳ Contains (0)
                       └── (empty)

        🛑
        ");
    }

    /// A new file nested in an existing directory: `/path/to/directory` is both the `from` path
    /// and the `to` path's parent, so today each half spells that shared directory out in full.
    #[test]
    fn from_to_to_nested_in_from() {
        let fixture = Fixture::new();
        let from = fixture.root().to_path_buf();
        let to = fixture.join("new.txt");

        let scrubber = fixture.scrub().not_found(&to);
        insta::assert_snapshot!(scrubber.carets(&FromTo::new(&from, &to).to_string()), @r"
        From path exists `/path/to/directory`
                                   ^^^^^^^^^
                                   ↳ Dir [✅ read, ✅ write, ✅ execute]
         - `/path/to/directory`
                  ^^
                  ↳ Dir [✅ read, ✅ write, ✅ execute]
                  ↳ Contains (1)
                    └── `directory` (exists)
        To path does not exist `/path/to/directory/new.txt`
                                                   ^^^^^^^
                                                   ↳ Missing: {error}
         - `/path/to/directory/new.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `new.txt`
                     ↳ Contains (0)
                       └── (empty)
        🛑
        ");
    }

    /// The other way around: the missing `from` file sits inside the existing `to` directory.
    /// Same shared directory, halves swapped.
    #[test]
    fn from_to_from_nested_in_to() {
        let fixture = Fixture::new();
        let from = fixture.join("new.txt");
        let to = fixture.root().to_path_buf();

        let scrubber = fixture.scrub().not_found(&from);
        insta::assert_snapshot!(scrubber.carets(&FromTo::new(&from, &to).to_string()), @r"
        From path does not exist `/path/to/directory/new.txt`
                                                     ^^^^^^^
                                                     ↳ Missing: {error}
         - `/path/to/directory/new.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `new.txt`
                     ↳ Contains (0)
                       └── (empty)
        To path exists `/path/to/directory`
                                 ^^^^^^^^^
                                 ↳ Dir [✅ read, ✅ write, ✅ execute]
         - `/path/to/directory`
                  ^^
                  ↳ Dir [✅ read, ✅ write, ✅ execute]
                  ↳ Contains (1)
                    └── `directory` (exists)
        🛑
        ");
    }
}

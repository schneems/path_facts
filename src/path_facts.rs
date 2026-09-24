//! Facts about paths
use crate::report::Report;
use std::{fmt::Display, path::Path};

/// Shows helpful facts about a path when `Display`ed.
///
/// See [`PathFacts::with_prefix`] for an example.
#[derive(Debug)]
pub struct PathFacts {
    _inner: Report,
    prefix: Option<String>,
}

impl PathFacts {
    /// Prefix path facts with a given string
    ///
    /// This is the recommended interface. As it allows us to directly annotate the path
    /// in the first line without having to repeat it like [`PathFacts::new`].
    ///
    /// The prefix input must either be the start of a line, or contain a newline for the caret spacing
    /// to work correctly.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use path_facts::PathFacts;
    ///
    /// # let path = std::path::Path::new("lol");
    /// let facts = PathFacts::with_prefix("Path", &path);
    /// eprintln!("{}", facts);
    /// ```
    ///
    /// Displays:
    ///
    ///```text
    #[doc = include_str!("snapshots/prior_dir_problem_is_file.txt")]
    ///```
    ///
    /// It can also be used with an empty prefix `""`. This allows us to annotate the path
    /// on the first line versus [`PathFacts::new`] doesn't know what will come before it,
    /// so it must repeat the path.
    pub fn with_prefix(prefix: impl AsRef<str>, path: impl AsRef<Path>) -> Self {
        let prefix = prefix.as_ref();
        let prefix = if prefix.is_empty() || prefix.ends_with(char::is_whitespace) {
            prefix.to_string()
        } else {
            format!("{prefix} ")
        };
        PathFacts {
            _inner: Report::new(path),
            prefix: Some(prefix),
        }
    }

    /// Display facts about a path
    ///
    /// This original interface was introduced before the caret (`^^^^`)
    /// underlining style was introduced. To support that style, it must repeat
    /// the path on the second line since it doesn't know if the first line has
    /// a prefix or not.
    ///
    /// To avoid this duplication, using [`PathFacts::with_prefix`] is recommended.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// use path_facts::PathFacts;
    ///
    /// # let path = std::path::Path::new("lol");
    /// let facts = PathFacts::new(&path);
    /// eprintln!("input {}", facts);
    /// ```
    ///
    /// Displays:
    ///
    ///```text
    #[doc = include_str!("snapshots/prior_dir_problem_orig.txt")]
    ///```
    pub fn new(path: impl AsRef<Path>) -> Self {
        PathFacts {
            _inner: Report::new(path),
            prefix: None,
        }
    }
}

impl Display for PathFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.prefix {
            None => self._inner.fmt(f),
            Some(prefix) => {
                writeln!(
                    f,
                    "{}",
                    self._inner
                        .render_with_prefix(prefix)
                        .trim_end_matches('\n')
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{snapshot_body, unix_newlines, Fixture};

    /// `PathFacts` is presentation only: it shows whatever the [`Report`] it holds shows.
    ///
    /// Every scenario is asserted once, against `Report`, in `report.rs`. This test is what makes
    /// those snapshots claims about `PathFacts` as well, so it renders one report and then hands
    /// that same value to the wrapper rather than walking the path twice. Two walks could disagree
    /// for reasons that have nothing to do with the forwarding this is here to pin.
    #[test]
    fn test_renders_its_report_verbatim() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();
        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        let report = Report::new(&path);
        let rendered = report.to_string();

        // Two empty strings would match as happily as two renderings, so say what was compared.
        assert!(
            rendered.contains("does not exist") && rendered.contains("Contains (1)"),
            "expected a report with facts in it to compare against:\n{}",
            rendered
        );
        assert_eq!(
            rendered,
            PathFacts {
                _inner: report,
                prefix: None
            }
            .to_string()
        );
    }

    #[test]
    fn with_prefix_forwards_to_report_render_with_prefix() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();
        let path = fixture.join("a.txt").join("b").join("does_not_exist.txt");

        let expected = Report::new(&path).render_with_prefix("Boop ");
        let actual = PathFacts::with_prefix("Boop ", &path).to_string();

        assert!(
            actual.contains("Boop does not exist") && actual.contains("File, not a dir"),
            "expected folded facts rendered after the prefix:\n{}",
            actual
        );
        assert_eq!(actual.trim_end(), expected.trim_end());
    }

    #[test]
    fn with_prefix_appends_a_space_when_the_prefix_has_no_trailing_whitespace() {
        let fixture = Fixture::new();
        let path = fixture.join("does_not_exist.txt");

        let rendered = PathFacts::with_prefix("Foo", &path).to_string();

        assert!(
            rendered.contains("Foo does not exist"),
            "expected one space between the lead and the facts:\n{}",
            rendered
        );
        assert_eq!(rendered, PathFacts::with_prefix("Foo ", &path).to_string());
    }

    #[test]
    fn with_prefix_keeps_trailing_whitespace_the_caller_wrote() {
        let fixture = Fixture::new();
        let path = fixture.join("does_not_exist.txt");

        let padded = PathFacts::with_prefix("Foo   ", &path).to_string();

        assert!(
            padded.contains("Foo   does not exist"),
            "expected the caller's three spaces kept verbatim:\n{}",
            padded
        );
        assert_ne!(padded, PathFacts::with_prefix("Foo ", &path).to_string());
    }

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
                fixture
                    .scrub()
                    .carets(&PathFacts::with_prefix("Path", &path).to_string())
            );
        });
    }

    #[test]
    fn test_prior_dir_problem_is_file_new() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();

        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::with_settings!({prepend_module_to_snapshot => false}, {
            insta::assert_snapshot!(
                "prior_dir_problem_orig",
                format!("input {}",
                    fixture
                        .scrub()
                        .carets(&PathFacts::new(&path).to_string())
                )
            );
        });
    }

    #[test]
    fn include_str_new_txt_file_matches_insta() {
        let include_str_file = unix_newlines(include_str!("snapshots/prior_dir_problem_orig.txt"))
            .trim()
            .to_string();
        let insta = snapshot_body(include_str!("snapshots/prior_dir_problem_orig.snap"));

        // Not `assert_eq!`: its `Debug` output escapes every newline, which turns a caret
        // misaligned by one column into two unreadable one-line blobs.
        assert!(
            include_str_file == insta,
            "the `PathFacts::with_prefix` doc example is no longer what the library renders. \
             Update `snapshots/prior_dir_problem_orig.txt` to match.\n\nGot:\n{}\n\n\
             Expected:\n{}\n",
            include_str_file,
            insta
        );
    }

    #[test]
    fn test_prior_dir_problem_is_file_with_prefix_empty() {
        let fixture = Fixture::new();
        std::fs::write(fixture.join("a.txt"), "").unwrap();

        let path = fixture
            .join("a.txt")
            .join("b")
            .join("c")
            .join("does_not_exist.txt");

        insta::assert_snapshot!(
            fixture.scrub().carets(&PathFacts::with_prefix("", &path).to_string()),
            @r"
        does not exist `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                                           ^^^^^
                                           ↳ File, not a dir [✅ read, ✅ write, ❌ execute]
         - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (1)
                       └── `a.txt` (exists)
        🛑
        "
        );
    }

    #[test]
    fn include_str_file_matches_insta() {
        let include_str_file =
            unix_newlines(include_str!("snapshots/prior_dir_problem_is_file.txt"))
                .trim()
                .to_string();
        let insta = snapshot_body(include_str!("snapshots/prior_dir_problem_is_file.snap"));

        // Not `assert_eq!`: its `Debug` output escapes every newline, which turns a caret
        // misaligned by one column into two unreadable one-line blobs.
        assert!(
            include_str_file == insta,
            "the `PathFacts::with_prefix` doc example is no longer what the library renders. \
             Update `snapshots/prior_dir_problem_is_file.txt` to match.\n\nGot:\n{}\n\n\
             Expected:\n{}\n",
            include_str_file,
            insta
        );
    }
}

//! Facts about paths
use crate::report::Report;
use std::{fmt::Display, path::Path};

/// Shows helpful facts about a path when `Display`ed.
#[derive(Debug)]
pub struct PathFacts {
    _inner: Report,
    /// Leading text rendered in front of the facts. `None` keeps the standalone two-bullet form.
    /// `Some` folds the caret and facts onto that first line.
    prefix: Option<String>,
}

impl PathFacts {
    pub fn new(path: impl AsRef<Path>) -> Self {
        PathFacts {
            _inner: Report::new(path),
            prefix: None,
        }
    }

    /// Renders the facts after `prefix`, folding the caret onto that first line.
    ///
    /// The character width of `prefix` shifts the caret to stay under the path, so a lead like
    /// `"Path "` no longer knocks it out of alignment. A `prefix` that does not already end in
    /// whitespace gains one space, so `"Path"` and `"Path "` render alike. Whitespace you write
    /// yourself is kept, so `"Path    "` stays padded. Plain [`new`](Self::new) keeps the
    /// two-bullet form.
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
    use crate::test_support::Fixture;

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
}

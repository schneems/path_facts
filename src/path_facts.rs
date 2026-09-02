//! Facts about paths
use crate::report::Report;
use std::{fmt::Display, path::Path};

/// Shows helpful facts about a path when `Display`ed.
#[derive(Debug)]
pub struct PathFacts {
    _inner: Report,
}

impl PathFacts {
    pub fn new(path: impl AsRef<Path>) -> Self {
        PathFacts {
            _inner: Report::new(path),
        }
    }
}

impl Display for PathFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self._inner.fmt(f)
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
        assert_eq!(rendered, PathFacts { _inner: report }.to_string());
    }
}

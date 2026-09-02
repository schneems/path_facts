use crate::PathFacts;
use std::{fmt::Display, path::Path};

/// Shows helpful facts about two paths when `Display`ed.
///
/// The output already contains "leader" text, i.e. `From path exists`. Versus a plain [`PathFacts`]
/// does not. For now this struct outputs the exact same information as if you had constructed two
/// [`PathFacts`], but in the future I hope to add some de-duplication logic.
///
/// Displaying this must either start at the beginning of a string or only immediately after a newline.
/// This reserves the right for adding a second line in the future that references the path in the first.
/// For example, carets `^^^^^^^` otherwise the second line indentation would be off.
pub struct FromTo {
    from: PathFacts,
    to: PathFacts,
}

impl FromTo {
    pub fn new(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Self {
        FromTo {
            from: PathFacts::new(from.as_ref()),
            to: PathFacts::new(to.as_ref()),
        }
    }
}

impl Display for FromTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "From path ")?;
        self.from.fmt(f)?;
        write!(f, "To path ")?;
        self.to.fmt(f)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;

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
}

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
#[derive(Debug)]
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
    use crate::test_support::{symlink_file, Fixture};

    /// Two spellings of one file: the `from` symlink `latest.log` resolves to the `to` path
    /// `2024-01.log`, its own target. Today the halves are independent — the `from` names its
    /// target and the `to` names the file, but neither says they are the same entry on disk.
    #[test]
    fn from_to_same_file_via_symlink() {
        let fixture = Fixture::new();
        let real = fixture.join("2024-01.log");
        std::fs::write(&real, "").unwrap();
        let link = fixture.join("latest.log");
        symlink_file("2024-01.log", &link).unwrap();

        insta::assert_snapshot!(fixture.scrub().carets(&FromTo::new(&link, &real).to_string()), @r"
        From path exists `/path/to/directory/latest.log`
         - `/path/to/directory/latest.log`
                               ^^^^^^^^^^
                               ↳ Symlink, resolves to file [✅ read, ✅ write, ❌ execute]
                               ↳ Target `2024-01.log` → `/path/to/directory/2024-01.log`
         - `/path/to/directory/latest.log`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ Contains (2)
                       ├── `2024-01.log`
                       └── `latest.log` (exists)
        To path exists `/path/to/directory/2024-01.log`
         - `/path/to/directory/2024-01.log`
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
         - `/path/to/directory/doesnotexist.txt`
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
         - `/path/to/directory`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
         - `/path/to/directory`
                  ^^
                  ↳ Dir [✅ read, ✅ write, ✅ execute]
                  ↳ Contains (1)
                    └── `directory` (exists)
        To path does not exist `/path/to/directory/new.txt`
         - `/path/to/directory/new.txt`
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
         - `/path/to/directory/new.txt`
                               ^^^^^^^
                               ↳ Missing: {error}
         - `/path/to/directory/new.txt`
                     ^^^^^^^^^
                     ↳ Dir [✅ read, ✅ write, ✅ execute]
                     ↳ ❌ Missing `new.txt`
                     ↳ Contains (0)
                       └── (empty)
        To path exists `/path/to/directory`
         - `/path/to/directory`
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

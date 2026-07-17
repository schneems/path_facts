use crate::{
    abs_path::AbsPath,
    happy_path::{DirOk, HappyPath},
};
use std::path::Path;

pub(crate) fn bullet(contents: impl AsRef<str>) -> String {
    prefix_first_rest_lines(" - ", "   ", contents.as_ref())
}

/// Shows permissions
pub(crate) fn permissions(read: bool, write: bool, execute: bool) -> String {
    let mut perms = vec![];
    perms.push(if read { "✅ read" } else { "❌ read" });
    perms.push(if write { "✅ write" } else { "❌ write" });
    perms.push(if execute {
        "✅ execute"
    } else {
        "❌ execute"
    });
    ["[", &perms.join(", "), "]"].join("").to_string()
}

/// Applies a prefix to the first line and a different prefix to the rest of the lines.
///
/// The primary use case is to align indentation with the prefix of the first line. Most often
/// for emitting indented bullet point lists.
///
/// The first prefix is always applied, even when the contents are empty. This default was
/// chosen to ensure that a nested-bullet point will always follow a parent bullet point,
/// even if that parent has no text.
pub(crate) fn prefix_first_rest_lines(
    first_prefix: &str,
    rest_prefix: &str,
    contents: &str,
) -> String {
    prefix_lines(contents, move |index, _| {
        if index == 0 {
            String::from(first_prefix)
        } else {
            String::from(rest_prefix)
        }
    })
}

/// Prefixes each line of input.
///
/// Each line of the provided string slice will be passed to the provided function along with
/// the index of the line. The function should return a string that will be prepended to the line.
///
/// If an empty string is provided, a prefix will still be added to improve UX in cases
/// where the caller forgot to pass a non-empty string.
pub(crate) fn prefix_lines<F: Fn(usize, &str) -> String>(contents: &str, f: F) -> String {
    // `split_inclusive` yields `None` for the empty string, so we have to explicitly add the prefix.
    if contents.is_empty() {
        f(0, "")
    } else {
        contents
            .split_inclusive('\n')
            .enumerate()
            .map(|(line_index, line)| {
                let prefix = f(line_index, line);
                if line == "\n" {
                    prefix.trim_end().to_string() + line
                } else {
                    prefix + line
                }
            })
            .collect()
    }
}

/// Shows a directory and files within it
///
/// If any of the entries are found within the directory they'll be annotated with information
/// based on their variant (either showing type of file and permissions or noting that it exists)
pub(crate) fn list_dir_with_files(dir: &DirOk, entries: &[PathInDir]) -> String {
    fmt_dir(dir, |entry| {
        entries.iter().find_map(|path| path.show_check(entry))
    })
}

/// Shows information about a path
///
/// If we can get file type and permissions from the path we show
/// maximal information, otherwise we have to scan entries in the dir to possibly annotate if
/// it exists or not
pub(crate) enum PathInDir<'a> {
    Exists(&'a HappyPath),
    MaybeExists(&'a AbsPath),
}

impl<'a> From<&'a HappyPath> for PathInDir<'a> {
    fn from(value: &'a HappyPath) -> Self {
        PathInDir::Exists(value)
    }
}

impl<'a> From<&'a AbsPath> for PathInDir<'a> {
    fn from(value: &'a AbsPath) -> Self {
        PathInDir::MaybeExists(value)
    }
}

impl<'a> PathInDir<'a> {
    fn show_check(&self, entry: &AbsPath) -> Option<String> {
        if entry == self.absolute() {
            match self {
                PathInDir::Exists(happy) => Some(format!(
                    "{} {}",
                    happy.resolved_type,
                    permissions(happy.read, happy.write, happy.execute)
                )),
                PathInDir::MaybeExists(_) => Some("(exists)".to_string()),
            }
        } else {
            None
        }
    }

    fn absolute(&'a self) -> &'a AbsPath {
        match self {
            PathInDir::Exists(happy_path) => &happy_path.absolute,
            PathInDir::MaybeExists(abs_path) => abs_path,
        }
    }
}

/// Shows a directory and it's permissions and ind the files inside of it
///
/// Yields each entry to the given annotation function, if it returns Some that annotation
/// is appended to the entry.
pub(crate) fn fmt_dir<F>(dir: &DirOk, annotate: F) -> String
where
    F: Fn(&AbsPath) -> Option<String>,
{
    format!(
        "{path}{perms}\n{entries}",
        path = dir.absolute,
        perms = if dir.read && dir.write && dir.execute {
            "".to_string()
        } else {
            format!(" {}", permissions(dir.read, dir.write, dir.execute))
        },
        entries = fmt_dir_entries_annotate(&dir.entries, annotate)
    )
}

/// Formats a vec of filenames
pub(crate) fn fmt_dir_entries_annotate<F>(entries: &[AbsPath], annotate: F) -> String
where
    F: Fn(&AbsPath) -> Option<String>,
{
    let mut out = String::new();
    if entries.is_empty() {
        out.push_str("   └── (empty)");
    } else {
        let mut iter = entries.iter().peekable();
        while let Some(subpath) = iter.next() {
            if let Some(name) = filename(subpath.as_ref()) {
                let mut entry = format!("`{}`", name);

                if let Some(annotation) = annotate(subpath) {
                    entry = format!("{entry} {annotation}");
                };

                if iter.peek().is_some() {
                    out.push_str(&format!("  ├── {entry}\n"));
                } else {
                    out.push_str(&format!("  └── {entry}\n"));
                }
            }
        }
    }
    out
}

pub(crate) fn filename(path: &Path) -> Option<std::path::Display<'_>> {
    path.file_name().map(|name| Path::new(name).display())
}

pub(crate) fn filename_or_path(path: &Path) -> std::path::Display<'_> {
    filename(path).unwrap_or_else(|| path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_empty_first_line_workd() {
        assert_eq!(prefix_first_rest_lines(" - ", "   ", "\n\n"), " -\n\n",);
    }

    #[test]
    fn prefix_does_not_indent_trailing_blank_line() {
        assert_eq!(
            prefix_first_rest_lines(" - ", "   ", "hello\n\n"),
            " - hello\n\n",
        );
    }
}

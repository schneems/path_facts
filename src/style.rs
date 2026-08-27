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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_empty_first_line_works() {
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

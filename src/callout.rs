//! Presentation layer for the callout-style path-facts format.
//!
//! A callout renders one path as a ``  - `path` `` bullet, draws a `^^^^` caret under the exact
//! component being discussed, and lists that component's facts beneath the caret. Each fact is
//! prefixed with `↳`. A directory's contents fold into a `↳ Contains (N)` fact whose tree is nested under it:
//!
//! ```text
//!  - `/tmp/lnk/../lnk/real/`
//!                     ^^^^
//!                     ↳ Dir [✅ read, ✅ write, ✅ execute]
//!                     ↳ Canonical `/private/tmp/lnk/real`
//!                     ↳ ❌ Missing `foo.txt`
//!                     ↳ Contains (3)
//!                       ├── `a`
//!                       ├── `b`
//!                       └── `c`
//! ```
//!
//! All column math is in **characters**, not bytes or terminal cells. Wide or zero-width glyphs
//! (`↳`, `✅`, `❌`, box-drawing) count as one character each, so a caret aligned under an ASCII
//! path stays exact while a fact line containing such glyphs may look shifted in a terminal that
//! renders them at a different cell width.

use crate::style;
use std::ops::Range;
use std::path::{Component, Path};

/// Columns before the path text on a bullet line: `" - "` (3 chars) plus the opening backtick.
const PATH_INDENT: usize = 4;

/// A caret span within a rendered path string, measured in characters (not bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) char_start: usize,
    pub(crate) char_width: usize,
}

/// Selects which component of a path the caret underlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Caret {
    /// The final component. Used for absolute/parent-directory callouts.
    Last,
    /// The component at this index into [`Path::components`]. Used for the leaf callout,
    /// anchored to the stopping step's recorded component index.
    Index(usize),
}

/// One path bullet, a caret aimed at one of its components, and the facts about that component.
#[derive(Debug, Clone)]
pub(crate) struct Callout {
    /// The path as displayed between backticks on the bullet line.
    pub(crate) path: String,
    /// Which characters of `path` the caret underlines. `None` draws no caret line and aligns
    /// facts under the path text.
    pub(crate) caret: Option<Span>,
    /// Facts about the component under the caret, rendered as `↳ …` lines beneath it.
    pub(crate) facts: Vec<Fact>,
}

/// A single `↳` line beneath a caret.
#[derive(Debug, Clone)]
pub(crate) enum Fact {
    /// A one- or multi-line `↳ {text}` fact. Continuation lines hang at `caret_col + 2`.
    Text(String),
    /// A directory listing folded into `↳ Contains (N)` with a tree nested at `caret_col + 2`.
    Contains { entries: Vec<Entry> },
}

/// One row of a [`Fact::Contains`] tree.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    /// The entry name, rendered between backticks.
    pub(crate) name: String,
    /// Optional trailing annotation, e.g. `directory [✅ read, …]`.
    pub(crate) annotation: Option<String>,
}

/// Locate the caret span for one component of `path`, in character columns.
///
/// The search walks [`Path::components`] and, for each, finds that component's text in the
/// displayed path starting from a running cursor that always advances past the previous match.
/// Searching the rendered string (rather than splitting on separators) keeps the caret correct
/// across a leading `./`, an interior `.` that `Components` drops, `..`, a trailing slash, and
/// repeated names — none of which line up with a naive `split('/')` index. Advancing the cursor
/// is what disambiguates repeats like `a/a`.
///
/// Component-wise lossy conversion equals the matching substring of the whole-path lossy
/// conversion because separators are single ASCII bytes that can never straddle a multi-byte
/// sequence, so the search is valid for non-UTF-8 paths too.
///
/// Returns `None` when the requested component does not exist (index out of range, or an empty
/// path for [`Caret::Last`]).
pub(crate) fn highlight(path: &Path, caret: Caret) -> Option<Span> {
    let full = path.display().to_string();
    let target_index = match caret {
        Caret::Index(index) => index,
        Caret::Last => path.components().count().checked_sub(1)?,
    };

    let mut byte_cursor = 0;
    for (index, component) in path.components().enumerate() {
        let found = find_component(&full, byte_cursor, component)?;
        if index == target_index {
            return Some(Span {
                char_start: full[..found.start].chars().count(),
                char_width: full[found.start..found.end].chars().count(),
            });
        }
        byte_cursor = found.end;
    }
    None
}

/// The byte range `component` occupies in `full`, searched forward from `from`.
///
/// Every component reports its own text except [`Component::RootDir`], which reports the
/// platform's preferred separator rather than the one the path was written with — always `\` on
/// Windows, which accepts `/` just as well. So searching for the reported text finds no root in
/// `C:/tmp/x`, and the whole path comes back with no caret at all. Matching whichever separator is
/// actually there keeps the cursor, and therefore every column after it, right on both spellings.
fn find_component(full: &str, from: usize, component: Component<'_>) -> Option<Range<usize>> {
    if component == Component::RootDir {
        // A separator is one ASCII byte, so its match is one byte and one character wide.
        let start = from + full[from..].find(std::path::is_separator)?;
        return Some(start..start + 1);
    }

    let text = component.as_os_str().to_string_lossy();
    let start = from + full[from..].find(text.as_ref())?;
    Some(start..start + text.len())
}

/// Render a sequence of callouts, one after another. The standalone summary line
/// (`` {verb} `{input}` ``) is prepended by the caller, not here, so a library consumer may put
/// arbitrary text in front of it.
pub(crate) fn render_callouts(callouts: &[Callout]) -> String {
    callouts
        .iter()
        .map(render_callout)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a single callout: the ``  - `path` `` bullet, the caret line (when there is one), then each
/// fact as a `↳` line aligned to the caret column.
pub(crate) fn render_callout(callout: &Callout) -> String {
    let caret_col = callout
        .caret
        .map_or(PATH_INDENT, |span| PATH_INDENT + span.char_start);

    let mut lines = vec![style::bullet(format!("`{}`", callout.path))];

    if let Some(span) = callout.caret {
        lines.push(format!(
            "{spaces}{carets}",
            spaces = " ".repeat(caret_col),
            carets = "^".repeat(span.char_width)
        ));
    }

    for fact in &callout.facts {
        push_fact(&mut lines, fact, caret_col);
    }

    lines.join("\n")
}

/// Append the line(s) for one fact, left-aligned to `caret_col`.
fn push_fact(lines: &mut Vec<String>, fact: &Fact, caret_col: usize) {
    let indent = " ".repeat(caret_col);
    match fact {
        Fact::Text(text) => lines.push(style::prefix_first_rest_lines(
            &format!("{indent}↳ "),
            &format!("{indent}  "),
            text,
        )),
        Fact::Contains { entries } => {
            lines.push(format!("{indent}↳ Contains ({})", entries.len()));
            let tree_indent = " ".repeat(caret_col + 2);

            // A count with no tree under it reads like the listing was cut off. Saying `(empty)`
            // in the shape of a tree row is what distinguishes "nothing is in here" from "nothing
            // was printed here".
            if entries.is_empty() {
                lines.push(format!("{tree_indent}└── (empty)"));
                return;
            }

            let mut iter = entries.iter().peekable();
            while let Some(entry) = iter.next() {
                let glyph = if iter.peek().is_some() {
                    "├──"
                } else {
                    "└──"
                };
                match &entry.annotation {
                    Some(annotation) => {
                        lines.push(format!(
                            "{tree_indent}{glyph} `{}` {annotation}",
                            entry.name
                        ));
                    }
                    None => lines.push(format!("{tree_indent}{glyph} `{}`", entry.name)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::module_doc_example;

    fn text(fact: &str) -> Fact {
        Fact::Text(fact.to_string())
    }

    fn entry(name: &str, annotation: Option<&str>) -> Entry {
        Entry {
            name: name.to_string(),
            annotation: annotation.map(String::from),
        }
    }

    /// The callout the module doc draws by hand at the top of this file.
    fn doc_example_callout() -> Callout {
        Callout {
            path: "/tmp/lnk/../lnk/real/".to_string(),
            caret: highlight(Path::new("/tmp/lnk/../lnk/real/"), Caret::Last),
            facts: vec![
                text("Dir [✅ read, ✅ write, ✅ execute]"),
                text("Canonical `/private/tmp/lnk/real`"),
                text("❌ Missing `foo.txt`"),
                Fact::Contains {
                    entries: vec![entry("a", None), entry("b", None), entry("c", None)],
                },
            ],
        }
    }

    #[test]
    fn module_doc_example_is_real_output() {
        // The module doc opens with the callout it describes, and has no other fenced block.
        let documented = module_doc_example(include_str!("callout.rs"), 0);
        let rendered = render_callout(&doc_example_callout());

        // Not `assert_eq!`: its `Debug` output escapes every newline, which turns a caret
        // misaligned by one column into two unreadable one-line blobs.
        assert!(
            documented == rendered,
            "the example at the top of `callout.rs` is no longer what the renderer \
             produces.\n\nDocumented:\n{}\n\nRendered:\n{}\n",
            documented,
            rendered
        );
    }

    #[test]
    fn highlight_leading_dot() {
        // `./hello` -> components [CurDir, hello]; the leading `.` becomes CurDir, so `hello`
        // sits at char 2 (`./`).
        assert_eq!(
            highlight(Path::new("./hello"), Caret::Last),
            Some(Span {
                char_start: 2,
                char_width: 5
            })
        );
    }

    #[test]
    fn highlight_interior_dot_is_dropped_from_components() {
        // `a/./b` -> components [a, b]; the interior `.` is NOT a component, but it IS in the
        // string, so `b` lands at char 4 — not the char-2 a `split('/')` index would give.
        assert_eq!(
            highlight(Path::new("a/./b"), Caret::Last),
            Some(Span {
                char_start: 4,
                char_width: 1
            })
        );
    }

    #[test]
    fn highlight_parent_dir() {
        // `link_out/..` -> the `..` component underlined.
        assert_eq!(
            highlight(Path::new("link_out/.."), Caret::Last),
            Some(Span {
                char_start: 9,
                char_width: 2
            })
        );
    }

    #[test]
    fn highlight_repeated_name_matches_second() {
        // `a/a` -> the cursor must advance past the first `a` so `Last` underlines the second.
        assert_eq!(
            highlight(Path::new("a/a"), Caret::Last),
            Some(Span {
                char_start: 2,
                char_width: 1
            })
        );
    }

    #[test]
    fn highlight_trailing_slash_from_spec() {
        // The spec's second callout: underline `real` in `/tmp/lnk/../lnk/real/`.
        assert_eq!(
            highlight(Path::new("/tmp/lnk/../lnk/real/"), Caret::Last),
            Some(Span {
                char_start: 16,
                char_width: 4
            })
        );
    }

    /// Windows accepts either separator in the same position, and `Component::RootDir` reports `\`
    /// for both, so the caret has to be found by looking at the path rather than at what the root
    /// component calls itself.
    #[test]
    #[cfg(windows)]
    fn highlight_windows_root_with_either_separator() {
        let expected = Some(Span {
            char_start: 18,
            char_width: 4,
        });

        assert_eq!(
            highlight(Path::new(r"C:\tmp\lnk\..\lnk\real"), Caret::Last),
            expected
        );
        assert_eq!(
            highlight(Path::new("C:/tmp/lnk/../lnk/real"), Caret::Last),
            expected
        );
    }

    #[test]
    fn highlight_by_index() {
        // Index refers to a `.components()` position, so index 1 skips the dropped interior `.`.
        assert_eq!(
            highlight(Path::new("a/./b"), Caret::Index(1)),
            Some(Span {
                char_start: 4,
                char_width: 1
            })
        );
        assert_eq!(highlight(Path::new("a/./b"), Caret::Index(9)), None);
    }

    #[test]
    fn highlight_empty_path_has_no_caret() {
        assert_eq!(highlight(Path::new(""), Caret::Last), None);
    }

    #[test]
    #[cfg(unix)]
    fn highlight_invalid_utf8_component() {
        use std::os::unix::ffi::OsStrExt;

        // `a/<0xff>/b`: the invalid byte renders as U+FFFD in both the component and the whole
        // path, so the forward search still locates the trailing `b` (char 4: a `/` � `/`).
        let raw = std::ffi::OsStr::from_bytes(b"a/\xff/b");
        assert_eq!(
            highlight(Path::new(raw), Caret::Last),
            Some(Span {
                char_start: 4,
                char_width: 1
            })
        );
    }

    #[test]
    fn render_dangling_symlink_callout() {
        let callout = Callout {
            path: "/tmp/lnk/dangling".to_string(),
            caret: highlight(Path::new("/tmp/lnk/dangling"), Caret::Last),
            facts: vec![
                text("Symlink, cannot follow: No such file or directory (os error 2)"),
                text("Target `../lnk/real/nope.txt` → `/tmp/lnk/../lnk/real/nope.txt`"),
            ],
        };

        insta::assert_snapshot!(render_callout(&callout), @r"
         - `/tmp/lnk/dangling`
                     ^^^^^^^^
                     ↳ Symlink, cannot follow: No such file or directory (os error 2)
                     ↳ Target `../lnk/real/nope.txt` → `/tmp/lnk/../lnk/real/nope.txt`
        ");
    }

    #[test]
    fn render_full_spec_callouts() {
        let leaf = Callout {
            path: "/tmp/lnk/dangling".to_string(),
            caret: highlight(Path::new("/tmp/lnk/dangling"), Caret::Last),
            facts: vec![
                text("Symlink, cannot follow: No such file or directory (os error 2)"),
                text("Target `../lnk/real/nope.txt` → `/tmp/lnk/../lnk/real/nope.txt`"),
            ],
        };
        let dir = doc_example_callout();

        insta::assert_snapshot!(render_callouts(&[leaf, dir]), @r"
         - `/tmp/lnk/dangling`
                     ^^^^^^^^
                     ↳ Symlink, cannot follow: No such file or directory (os error 2)
                     ↳ Target `../lnk/real/nope.txt` → `/tmp/lnk/../lnk/real/nope.txt`
         - `/tmp/lnk/../lnk/real/`
                            ^^^^
                            ↳ Dir [✅ read, ✅ write, ✅ execute]
                            ↳ Canonical `/private/tmp/lnk/real`
                            ↳ ❌ Missing `foo.txt`
                            ↳ Contains (3)
                              ├── `a`
                              ├── `b`
                              └── `c`
        ");
    }

    #[test]
    fn render_empty_contains_says_so() {
        let callout = Callout {
            path: "/tmp/empty".to_string(),
            caret: highlight(Path::new("/tmp/empty"), Caret::Last),
            facts: vec![Fact::Contains { entries: vec![] }],
        };

        insta::assert_snapshot!(render_callout(&callout), @r"
         - `/tmp/empty`
                 ^^^^^
                 ↳ Contains (0)
                   └── (empty)
        ");
    }

    #[test]
    fn render_multiline_fact_hangs_under_text() {
        let callout = Callout {
            path: "/tmp/x".to_string(),
            caret: highlight(Path::new("/tmp/x"), Caret::Last),
            facts: vec![text("Missing `x` from parent directory:\n`/tmp`")],
        };

        insta::assert_snapshot!(render_callout(&callout), @r"
         - `/tmp/x`
                 ^
                 ↳ Missing `x` from parent directory:
                   `/tmp`
        ");
    }

    #[test]
    fn render_callout_without_caret_aligns_facts_under_path() {
        let callout = Callout {
            path: "/".to_string(),
            caret: None,
            facts: vec![text("is root")],
        };

        insta::assert_snapshot!(render_callout(&callout), @r"
         - `/`
            ↳ is root
        ");
    }

    #[test]
    fn render_entry_annotation() {
        let callout = Callout {
            path: "/tmp/d".to_string(),
            caret: highlight(Path::new("/tmp/d"), Caret::Last),
            facts: vec![Fact::Contains {
                entries: vec![
                    entry("a.txt", Some("file [✅ read, ✅ write, ❌ execute]")),
                    entry("b", None),
                ],
            }],
        };

        insta::assert_snapshot!(render_callout(&callout), @r"
         - `/tmp/d`
                 ^
                 ↳ Contains (2)
                   ├── `a.txt` file [✅ read, ✅ write, ❌ execute]
                   └── `b`
        ");
    }
}

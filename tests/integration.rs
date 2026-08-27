//! Integration coverage
//!
//! Most tests are unit tests. This one is here because it is the only test that runs the crate the
//! way a dependent gets it: unit tests compile with `cfg(test)` on, and the doctests that link
//! against the real build are `no_run`. Behavior that differs between the two configurations is
//! invisible to everything else — see the `read_dir` sort in `abs_path.rs` for the trap.
use path_facts::PathFacts;
use std::path::Path;

const PLACEHOLDER: &str = "/TMP";

const STOP: &str = "🛑";

/// Rewrites the tempdir prefix out of `rendered`, keeping every caret over the characters it
/// actually points at.
fn scrub(rendered: &str, root: &Path) -> String {
    let root = root.display().to_string();
    let dedent = root.chars().count() - PLACEHOLDER.chars().count();

    let scrubbed = rendered
        .lines()
        .map(|line| {
            if line.contains(&root) {
                return line.replace(&root, PLACEHOLDER);
            }
            let indent = line.chars().take_while(|char| *char == ' ').count();
            assert!(
                indent >= dedent,
                "cannot dedent by {}, line points inside the scrubbed prefix: {:?}",
                dedent,
                line
            );
            line.chars().skip(dedent).collect()
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("{scrubbed}\n{STOP}")
}

#[test]
fn walks_into_a_file_and_stops() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    // Named rather than the tempdir root itself: the parent block's caret lands on this
    // component, and a random `.tmpXXXXXX` would be scrubbed out from under it.
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();

    std::fs::write(project.join("a.txt"), "hello").unwrap();
    std::fs::write(project.join("sibling.txt"), "hi").unwrap();

    let input = project.join("a.txt").join("b").join("nope.txt");
    let rendered = PathFacts::new(&input).to_string();

    insta::assert_snapshot!(scrub(&rendered, &root), @r"
    does not exist `/TMP/project/a.txt/b/nope.txt`
     - `/TMP/project/a.txt/b/nope.txt`
                     ^^^^^
                     ↳ File, not a dir [✅ read, ✅ write, ❌ execute]
     - `/TMP/project/a.txt/b/nope.txt`
             ^^^^^^^
             ↳ Dir [✅ read, ✅ write, ✅ execute]
             ↳ Contains (2)
               ├── `a.txt` (exists)
               └── `sibling.txt`
    🛑
    ");
}

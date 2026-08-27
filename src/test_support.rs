//! Fixtures and output scrubbing for the tests that render a path.
//!
//! A rendering test wants directories with stable names on disk, the platform workarounds to build
//! them, and the tempdir prefix scrubbed back out of what it recorded. [`Report`](crate::Report)
//! holds nearly all of those tests, but the fixtures live here rather than beside them so a test
//! elsewhere in the crate can set up the same filesystem without copying the workarounds.
//!
//! [`module_doc_example`] and [`snapshot_body`] are here for a related reason. Several files paste
//! rendered output into their documentation, and each one wants to prove its paste is still what
//! the renderer produces. That comparison needs the doc read back out of the source and the
//! recorded output read back out of a snapshot file, which is the same two readers every time.

use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Marks the end of a snapshot so a trailing newline is visible rather than trimmed away by an
/// editor, a diff, or the eye.
pub(crate) const STOP: &str = "🛑";

/// The body of the `index`-th fenced block in a file's module doc, with the `//!` prefixes gone.
///
/// Takes the source text rather than a path because `include_str!` only accepts a literal, so a
/// file is the only place that can name itself.
///
/// Blocks are counted in source order, which is the only handle a caller has: the fence's info
/// string is `text` on every block worth checking, so it cannot tell two of them apart.
pub(crate) fn module_doc_example(source: &str, index: usize) -> String {
    const FENCE: &str = "```";

    let mut blocks = Vec::new();
    let mut open: Option<Vec<&str>> = None;

    // `map_while` stops at the first line that is not module doc, so a `//!` appearing later in
    // the file (inside a string, say) cannot be mistaken for more documentation.
    for line in source.lines().map_while(|line| line.strip_prefix("//!")) {
        if line.trim_start().starts_with(FENCE) {
            match open.take() {
                Some(block) => blocks.push(block.join("\n")),
                None => open = Some(Vec::new()),
            }
        } else if let Some(block) = open.as_mut() {
            block.push(line.strip_prefix(' ').unwrap_or(line));
        }
    }

    blocks
        .into_iter()
        .nth(index)
        .unwrap_or_else(|| panic!("module doc has no fenced block at index {}", index))
}

/// The output an `insta` snapshot file records, without the YAML frontmatter or the [`STOP`]
/// marker the suite appends.
///
/// `splitn` rather than `split` so a `---` inside the recorded output keeps the rest of it: the
/// frontmatter is the first two fences and everything after them is the body.
pub(crate) fn snapshot_body(snapshot: &str) -> String {
    snapshot
        .splitn(3, "---")
        .nth(2)
        .expect("snapshot should have YAML frontmatter")
        .replace(STOP, "")
        .trim()
        .to_string()
}

/// A tempdir whose fixture root is spelled `/path/to/directory` once scrubbed.
///
/// The `path/to/directory` part is real on disk rather than a placeholder substituted in
/// afterwards. A callout points a caret at a component of the path it prints, and the tempdir's
/// own `.tmpA1b2C3` is both random and exactly what the parent callout's caret tends to land on — so
/// rewriting it after rendering would erase the thing the caret names. Walking real directories
/// with stable names gives every caret something to point at and leaves only the tempdir prefix
/// above them to strip.
pub(crate) struct Fixture {
    /// Held for its `Drop`, which removes the directory tree.
    _temp: tempfile::TempDir,
    /// The canonical tempdir, which is the prefix scrubbed out of rendered output.
    anchor: PathBuf,
    /// `<anchor>/path/to/directory`, where fixtures are built.
    root: PathBuf,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Fixture::named("directory")
    }

    /// A fixture whose root is spelled `/path/to/{name}`, for a test that needs two of them far
    /// enough apart that no relative path could reach from one to the other.
    pub(crate) fn named(name: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let anchor = temp.path().canonicalize().unwrap();
        let root = anchor.join("path").join("to").join(name);
        std::fs::create_dir_all(&root).unwrap();

        Fixture {
            _temp: temp,
            anchor,
            root,
        }
    }

    /// The directory fixtures are built in, spelled `/path/to/directory` after scrubbing.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The tempdir prefix this fixture's scrubber strips, for chaining onto another's.
    pub(crate) fn anchor(&self) -> &Path {
        &self.anchor
    }

    pub(crate) fn join(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Makes the fixture root the process-wide current directory, for relative-path cases.
    ///
    /// Safe only because the suite runs under nextest, which gives each test its own process.
    pub(crate) fn enter(&self) {
        std::env::set_current_dir(&self.root).unwrap();
    }

    /// A scrubber that already knows to strip the tempdir prefix.
    pub(crate) fn scrub(&self) -> Scrubber {
        Scrubber::new().path(&self.anchor, "")
    }
}

/// Rewrites machine-specific text out of rendered output and keeps every caret aligned.
///
/// Replacing a path with a shorter placeholder slides a bullet line left while the `^^^^` and `↳`
/// lines beneath it stay where they were, so a correct caret reads as broken. Padding the
/// placeholder back out to the original width fixes the columns but then hides whatever the caret
/// was pointing at behind the padding. Neither is a snapshot worth reading.
///
/// So both sides move: [`Scrubber::carets`] shifts each line's indentation by however much the
/// bullet above it grew or shrank.
pub(crate) struct Scrubber {
    rules: Vec<(String, String)>,
}

impl Scrubber {
    pub(crate) fn new() -> Self {
        Scrubber { rules: Vec::new() }
    }

    /// Replaces `from` with `to` wherever it appears.
    pub(crate) fn path(mut self, from: impl AsRef<Path>, to: &str) -> Self {
        // A verbatim prefix is stripped from the whole line before any rule runs, so the rule has
        // to be stored stripped too or it will never match on Windows.
        let from = from.as_ref().display().to_string().replace(r"\\?\", "");
        self.rules.push((from, to.to_string()));
        self
    }

    /// Spells the filesystem root `path` hangs off `/`, for output that names a root itself.
    ///
    /// A root is `/` on unix and a drive like `C:\` on Windows, so a snapshot that shows one can
    /// only be shared across platforms if the two are rewritten to the same thing.
    ///
    /// Add this after every [`Scrubber::path`] rule. On Windows a root is the front of *every*
    /// absolute path, so a longer prefix has to be stripped before this rule gets a look at the
    /// line.
    pub(crate) fn root_of(self, path: impl AsRef<Path>) -> Self {
        let root = path
            .as_ref()
            .components()
            .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
            .map(|component| component.as_os_str())
            .collect::<PathBuf>();

        self.path(root, "/")
    }

    /// Replaces an OS error message with `{error}`, whose wording varies by platform and libc.
    pub(crate) fn error(mut self, error: std::io::Error) -> Self {
        self.rules.push((error.to_string(), "{error}".to_string()));
        self
    }

    /// Scrubs output that carries carets, realigning them, and marks the end with [`STOP`].
    pub(crate) fn carets(&self, rendered: &str) -> String {
        let mut shift = 0isize;
        let mut lines = Vec::new();

        for line in rendered.lines() {
            let rewritten = self.rewrite(line);
            if line.starts_with(" - ") || !line.starts_with(' ') {
                // A bullet (or the summary line): whatever the path lost, the caret and fact lines
                // hanging beneath it have to lose from their indentation.
                shift = width(line) - width(&rewritten);
                lines.push(rewritten);
            } else {
                lines.push(reindent(&rewritten, shift));
            }
        }

        format!("{}\n{STOP}", lines.join("\n"))
    }

    /// Scrubs output that carries no carets, so nothing needs realigning.
    pub(crate) fn plain(&self, rendered: &str) -> String {
        let scrubbed = rendered
            .lines()
            .map(|line| self.rewrite(line))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{scrubbed}\n{STOP}")
    }

    fn rewrite(&self, line: &str) -> String {
        // Windows `canonicalize` yields a `\\?\` verbatim prefix but `read_link` does not, so one
        // rendered line can hold both spellings of the same directory. Strip it first and the two
        // become one string the rules can match.
        let mut out = line.replace(r"\\?\", "");
        for (from, to) in &self.rules {
            out = out.replace(from, to);
        }
        out.replace('\\', "/")
    }
}

fn width(line: &str) -> isize {
    line.chars().count() as isize
}

/// Moves a line left or right by `shift` columns of leading space.
fn reindent(line: &str, shift: isize) -> String {
    if shift <= 0 {
        return " ".repeat(shift.unsigned_abs()) + line;
    }

    let shift = shift as usize;
    let indent = line.chars().take_while(|char| *char == ' ').count();
    // Only reachable if a caret points inside the text being scrubbed away, which leaves nothing
    // for it to name. Build the fixture under `Fixture::root` so every caret lands to the right
    // of the tempdir prefix.
    assert!(
        indent >= shift,
        "cannot dedent by {}, line points inside the scrubbed prefix: {:?}",
        shift,
        line
    );
    line.chars().skip(shift).collect()
}

// Cross-platform symlink creation. Unix has a single `symlink` that ignores the target's type;
// Windows splits it into `symlink_file` and `symlink_dir` and needs the right one chosen up front.
//
// Creating a symlink on Windows requires SeCreateSymbolicLinkPrivilege (admin or Developer Mode,
// which GitHub's runners enable).
pub(crate) fn symlink_file<P: AsRef<Path>, Q: AsRef<Path>>(
    target: P,
    link: Q,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}

pub(crate) fn symlink_dir<P: AsRef<Path>, Q: AsRef<Path>>(
    target: P,
    link: Q,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
}

// Cross-platform "remove write permission from a directory". Unix drops the write mode bit
// (keeping read+execute so the directory is still traversable).
//
// Windows ignores the read-only *attribute* on directories, and `faccess`'s directory path skips
// that attribute entirely — it evaluates the real DACL via the Win32 `AccessCheck` API. So
// `set_readonly(true)` would be a no-op for the write check. Instead we add an explicit deny-write
// ACE for the current user with `icacls /deny`. Unlike a traverse (execute) deny, a DACL
// deny-write ACE is honored by `AccessCheck` even under the admin token the CI runner uses, so
// this makes `access(WRITE)` report the directory as not writable.
//
// We deny the granular write rights (WD,AD,WEA,WA) rather than the `(W)` simple-rights alias.
// `(W)` maps to `FILE_GENERIC_WRITE`, which shares `READ_CONTROL` and `SYNCHRONIZE` with
// `FILE_GENERIC_EXECUTE` — denying those collaterally fails faccess's `EXECUTE` check. The
// granular deny touches only write-data/append/EA/attr rights, so execute still reads `✅`,
// matching the Unix 0o555 behavior.
//
// The deny ACE persists on the directory; `restore_write` removes it so the tempdir can be
// cleaned up.
pub(crate) fn set_read_only<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        set_mode(path, 0o555) // read + execute, no write
    }
    #[cfg(windows)]
    {
        icacls(
            path.as_ref(),
            "/deny",
            &format!("{}:(WD,AD,WEA,WA)", current_user()?),
        )
    }
}

// Undo the deny-write ACE added by `set_read_only` on Windows so the tempdir can be removed.
// No-op on unix, where `tempfile` can clean up a mode-0o555 directory because the *parent* is
// still writable.
#[cfg(windows)]
pub(crate) fn restore_write<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    icacls(path.as_ref(), "/remove:d", &current_user()?)
}

#[cfg(windows)]
fn current_user() -> std::io::Result<String> {
    std::env::var("USERNAME")
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "USERNAME not set"))
}

#[cfg(windows)]
fn icacls(path: &Path, flag: &str, spec: &str) -> std::io::Result<()> {
    let output = std::process::Command::new("icacls")
        .arg(path)
        .arg(flag)
        .arg(spec)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "icacls {flag} {spec} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

// Unix-only mode helper: sets specific POSIX mode bits. The tests that use it to remove directory
// execute/traverse are `#[cfg(unix)]`; see those tests for why the behavior can't be reproduced on
// the Windows CI runner.
#[cfg(unix)]
pub(crate) fn set_mode<P: AsRef<Path>>(path: P, mode: u32) -> std::io::Result<()> {
    let path = path.as_ref();
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms)
}

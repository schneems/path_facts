# Changelog

## Unreleased

- Add: `PathFacts::with_prefix(prefix, path)` renders the facts after `prefix` and folds the caret and facts onto that first line instead of repeating the path on a bullet below it. The width of `prefix` keeps the caret aligned under the path, so a lead like `Path ` no longer shifts it. A `prefix` with no trailing whitespace gains a single space so it does not run into the facts (`"Path"` and `"Path "` render alike), while whitespace you write yourself is kept as-is. Plain `PathFacts::new` keeps the previous two-bullet form.

Before, prepending a lead by hand (`format!("Path {}", PathFacts::new(&path))`) repeated the full path on a bullet below the summary:

```
Path does not exist `/path/to/directory/a.txt/b/c/does_not_exist.txt`
 - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                       ^^^^^
                       ↳ File, not a dir [✅ read, ✅ write, ❌ execute]
 - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
             ^^^^^^^^^
             ↳ Dir [✅ read, ✅ write, ✅ execute]
             ↳ Contains (1)
               └── `a.txt` (exists)
```

After (`PathFacts::with_prefix("Path ", &path)`), the facts fold onto the summary with the caret still under the path:

```
Path does not exist `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                                        ^^^^^
                                        ↳ File, not a dir [✅ read, ✅ write, ❌ execute]
 - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
             ^^^^^^^^^
             ↳ Dir [✅ read, ✅ write, ✅ execute]
             ↳ Contains (1)
               └── `a.txt` (exists)
```

- Change: `FromTo` folds each half's caret and facts onto its `From path`/`To path` summary line rather than repeating the path on a bullet beneath it.

- Add: When a `FromTo`'s two paths resolve to the same location on disk, the `from` half now says so and points down at the `to` half below it. Detection is by resolved location, so it holds across a symlink and its target, a folded `..`, and a relative path spelled against an absolute one.

```
From path exists `/path/to/directory/latest.log`
                                     ^^^^^^^^^^
                                     ↳ Symlink, resolves to file [✅ read, ✅ write, ❌ execute]
                                     ↳ Target `2024-01.log` → `/path/to/directory/2024-01.log`
                                     ↳ Same location as to path (below)
...
To path exists `/path/to/directory/2024-01.log`
```

- Fix: Previously, if an input was a relative path like `doesnotexist.txt`, then its directory listing would not include the filename.

Before:

```
 - `/path/to/directory`
             ^^^^^^^^^
             ↳ Dir [✅ read, ✅ write, ✅ execute]
             ↳ ❌ Missing `doesnotexist.txt`
             ↳ Contains (0)
               └── (empty)
```

After this change, it includes the filename:

```
 - `/path/to/directory/doesnotexist.txt`
             ^^^^^^^^^
             ↳ Dir [✅ read, ✅ write, ✅ execute]
             ↳ ❌ Missing `doesnotexist.txt`
             ↳ Contains (0)
               └── (empty)
```

## 0.3.0

- Add `#[derive(Debug)]` on public structs
- Introduce `path_facts::FromTo` struct. Use to construct and hold information about a directional pair of paths.
- Update the display interface with a new `^^^^` caret feature for highlighting the part of the path
  we are referencing.

```
does not exist `/path/to/directory/a.txt/b/c/does_not_exist.txt`
 - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
                       ^^^^^
                       ↳ File, not a dir [✅ read, ✅ write, ❌ execute]
 - `/path/to/directory/a.txt/b/c/does_not_exist.txt`
             ^^^^^^^^^
             ↳ Dir [✅ read, ✅ write, ✅ execute]
             ↳ Contains (1)
               └── `a.txt` (exists)
```

- Directory contents are now sorted by name which matches `cargo package --list`
- Fix bug with readlink. A symlink such as `/a/b/c` → `d/e/f` (relative symlink) will be joined to the
  dir of the file it's in, so the system would read it as `/a/b/d/e/f`. Previously this incorrectly reported
  it was `/a/b/c/d/e/f`.
- Fix the parent-directory listing for a path ending in `..`. The listing is now built from the directory
  the walk physically resolves, rather than the lexical parent of the un-normalized
  absolute path. Previously `/path/to/directory/a/b/..` listed `/path/to/directory/a/b` and failed to
  annotate the resolved entry, because the annotation compared directory entries against the un-normalized
  absolute path, which no real entry equals. Entries are now compared against the resolved entry as it
  appears in that directory's listing.

Before:

```
exists `/path/to/directory/a/b/..`
 - `/path/to/directory/a/b`
     └── `inside_b.txt`
```

After:

```
exists `/path/to/directory/a/b/..`
 - `/path/to/directory`
     ├── `a` directory [✅ read, ✅ write, ✅ execute]
     └── `inside_dir.txt`
```

## 0.2.2

- Fix inconsistent trailing newline. Previously some facts ended with one newline and some with two. They now consistently end with a single trailing newline, so interpolating a fact into a larger message no longer injects an extra blank line.
- Improve output when prior path is not a directory

Before:

```
cannot access `/path/to/directory/a/b/c/does_not_exist.txt`
 - Prior path is not a directory
 - Prior path exists `/path/to/directory/a`
    - `/path/to/directory`
        └── `a` file [✅ read, ✅ write, ❌ execute]
```

After:

```
cannot access `/path/to/directory/a/b/c/does_not_exist.txt`
 - Prior path is not a directory `/path/to/directory/a`
    - `/path/to/directory`
        └── `a` file [✅ read, ✅ write, ❌ execute]
```

## 0.2.1

- Fix: Always emit permissions for main file/directory [#9](https://github.com/schneems/path_facts/pull/9). Previously we skipped emitting permissions when RWX were all true, but it looked off in some scenarios:

```
                  ├── `link_to_dir` directory []
```

Fixed to always output the permissions for the target file/directory:

```
                  ├── `link_to_dir` directory [✅ read, ✅ write, ✅ execute]
```

## 0.2.0

- Add: Minimum supported Rust version (MSRV) constraint as 1.79, which is when [`std::path::absolute`](https://doc.rust-lang.org/std/path/fn.absolute.html) was stabilized [#8](https://github.com/schneems/path_facts/issues/8).

## 0.1.0

- Initial implementation

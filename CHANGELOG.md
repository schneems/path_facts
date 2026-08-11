# Changelog

## Unreleased

- Add expanded path. Previously the library used absolute paths as a common demonimator. However the
  `std::fs::absolute` does not remove parent parts (`..`) and current dir `.` so two paths could represent
  the same path on disk, but have two different representations. An "expanded" path is either a canonical
  path (if the file/dir exists) or the parts of the prior directories that exist (and can be canonicalized)
  with the remainder that cannot be. This transformation is now shown as an arrow on the top line.

```
exists `exists.txt` → `/path/to/directory/exists.txt`
 - `/path/to/directory`
     └── `exists.txt` file [✅ read, ✅ write, ❌ execute]
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

# Changelog

## Unreleased

- Normalize absolute path output by removing `.` and `..` parts.

Before:

```
cannot access `../c/does_not_exist.txt`
 - Absolute: `/path/to/directory/a/../c/does_not_exist.txt`
```

After:

```
cannot access `../c/does_not_exist.txt`
 - Absolute: `/path/to/directory/c/does_not_exist.txt`
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

# Path Facts

Thanks for subscribing to PATH FACTS. You'll get a fun fact every \<15 minutes\> reply "TWO ROADS DIVERGED" to unsubscribe.

## What

Unlike the world fameous `cat` FACTS, path facts only appear when you ask them, and are best right after a `std::io::Error`.

The purpose of path facts is to deliver maximum information about your system on disk in a tidy, easy to understand package. The idea is that it gives you enough information about the state of your disk to debug an unexpected error without requiring you to run around calling `ls` and `cat` until you find the problem.

Tired of seeing this:

```text
No such file or directory
```

When you could be seeing this?

```text
cannot access `/path/to/directory/a/b/c/does_not_exist.txt`
 - Prior path is not a directory
 - Prior path exists `/path/to/directory/a`
 - `/path/to/directory`
     └── `a` (file: ✅ read, ✅ write, ❌ execute)
```

Then start using path facts today!

## No, but really. Why?

I designed this library to be used by other libraries that extend or mimic `std::fs` so they can create beautiful errors. Of course, you don't have to be a library author to get your PATH FACTS fix.

The philosophy behind PATH FACTS is that our systems shouldn't just tell us when something goes wrong, but they should be able to describe WHY it went wrong in a way that you (the developer) can do something about it. That's the same philosophy I used to introduce a parsing-agnostic [syntax suggesting algorithm into Ruby core](https://github.com/ruby/syntax_suggest).

## Use

The API is pretty simple. Construct a `PathFact` with a path and `Display` it as you like:

```rust,no_run
use path_facts::PathFacts;

let path = std::path::Path::new("doesnotexist.txt");
std::fs::read_to_string(&path)
    .map_err(|error| format!("{error}. {}", PathFacts::new(&path)))
    .unwrap();
```

For an operation with multiple paths you can use multiple PATH FACTS structs. For example:

```rust,no_run
use path_facts::PathFacts;

let from = std::path::Path::new("doesnotexist.txt");
let to = std::path::Path::new("also_does_not_exist.txt");
std::fs::rename(&from, to)
    .map_err(|error| {
        format!(
            "cannot rename from `{}` to `{}` due to: {error}.\nFrom path {from_facts}To path {to_facts}",
            from.display(),
            to.display(),
            from_facts = PathFacts::new(&from),
            to_facts = PathFacts::new(&to)
        )
    }).unwrap();
```

## Considerations

- Not everyone likes facts: If you add this to your library, consider adding a feature to enable/disable it. We recommend the feature name `path_facts`.
- Facts don't come cheap: This library will make system calls. If performance is a concern, don't call `PathFacts::new` on a hot codepath. Instead you should store the path and lazilly call `PathFacts` only when the error is rendered. We assume that computers do stuff fast and developers do stuff slowly. You'll be trading off some compute time to reduce end developer debugging time.
- Top secret facts: If your errors end up being displayed to a user and they can manipulate the input then they can already guess and check what files are present on your system. If you introduce this library then an attacker could gain more information (such as specific file permissions) and make it easier to list directory contents. We recommend using this library in contexts where developer logs are kept separate from user facing errors or where the user already has access to the entire disk (such as in a Cloud Native Buildpack).

## But I came here for silly path facts, not a serious library

Fine. I searched "path facts" and the closest I found was [top 10 facts about roads](https://www.funkidslive.com/learn/top-10-facts/top-ten-facts-about-roads/).

Did you know that the first roads were made by animals? Amazing!

For more (serious, and not silly) PATH FACTS `cargo add path_facts` today!

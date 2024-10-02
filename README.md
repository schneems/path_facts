# Path Facts

Thanks for subscribing to PATH FACTS. You'll get a fun fact every \<15 minutes\> reply "TWO ROADS DIVERGED" to unsubscribe!

## What

Unlike the world-famous `cat` FACTS, path facts only appear when you ask them and are best right after a `std::io::Error` from a filesystem operation.

The purpose of path facts is to deliver maximum information about your system on disk in a tidy, easy-to-understand package. The idea is that it gives you enough information to debug an unexpected error without requiring you to run around calling `ls` and `cat` until you find the problem.

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

The philosophy behind PATH FACTS is that our systems shouldn't just tell us when something goes wrong, but they should be able to describe WHY it went wrong so that you (the developer) can do something about it. That's the same philosophy I used to introduce a parsing-agnostic [syntax suggesting algorithm into Ruby core](https://github.com/ruby/syntax_suggest).

## Use

The API is small. Construct a `PathFact` with a path and `Display` it as you like:

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
use indoc::formatdoc;

let from = std::path::Path::new("doesnotexist.txt");
let to = std::path::Path::new("also_does_not_exist.txt");
std::fs::rename(&from, to)
    .map_err(|error| {
        formatdoc! {"
            cannot rename from `{}` to `{}` due to: {error}.
            From path {from_facts}
            To path {to_facts}
            ",
            from.display(),
            to.display(),
            from_facts = PathFacts::new(&from).to_string().trim(),
            to_facts = PathFacts::new(&to)
        }.trim().to_string()
    }).unwrap();
```

## Actual path facts

Here are a few facts about paths that some people might find interesting. If you're staring at path facts and an error message, maybe one of these tidbits could help you connect the dots:

- Fact: Failing to access a path does not guarantee it doesn't exist. It might mean you don't have permission even to know whether it exists.
- Fact: If a parent directory of a path does not exist, the path does not exist!
- Fact: It is possible to read file permissions of a file you cannot "read" (if the directory it's in has effective `execute` permissions).
- Fact: Files in a directory can be readable and modifiable, but if the directory is missing the `execute` permission, you're not allowed to view the metadata (such as permission info).
- Fact: Deleting and creating files depends on whether the directory they're in has the `write` permission or not!
- Fact: Parent directories cannot be files (unless they're valid symlinks or hard links to a valid directory)!
- Fact: Making an infinitely recursive structure of paths using symlinks is possible. (FYI, this is why we don't try to follow broken symlinks to completion)
- Fact: Permissions of a path depend not just on the permissions of the specific file/directory but also on other things, such as inherited permissions from parent directories.
  - This means that to know the "effective" permissions of a file, you need to know the permissions of all its parent directories (we use the `faccess` crate for this)
  - More permissions info at https://www.redhat.com/sysadmin/linux-file-permissions-explained and https://www.redhat.com/sysadmin/suid-sgid-sticky-bit
- Fact: Different operating systems have different permissions models. Even on Linux, there are additional ways to restrict file capabilities, such as Access Control Lists (ACLs).
  - This library is OS independent but prioritizes posix systems (Linux, Mac) and, to a lesser degree, Windows.
- Fact: The first paths were made by animals. Source: [top 10 facts about ~~paths~~ roads](https://www.funkidslive.com/learn/top-10-facts/top-ten-facts-about-roads/)

## Usage considerations

If you're considering adopting this library, here are some things you should consider:

- TOC/TOU: This stands for "time of check/time of use," meaning there are race conditions when dealing with files. Files on disk are effectively a distributed system there's no way for this library to guarantee that the disk was not modified between when your original command failed and when this library ran. This library tries to gather information on disk as atomically as it can, but ultimately, it must assume that the prior information collected is still valid. Does that mean that our path facts could be path fiction? Anything we print was true at some time. It's the user's responsibility to be aware of how their system is accessed and modified and act accordingly.
- Not everyone likes facts: If you add this to your library, consider adding a feature to enable/disable it. We recommend the feature name `path_facts`.
- Facts don't come cheap: This library will make system calls. If performance is a concern, don't call `PathFacts::new` on a hot code path. Instead, you could store the path and lazily call `PathFacts` only when the error is rendered. We assume that computers do stuff fast and developers do stuff slowly. You'll be trading off some compute time to reduce end developer debugging time.
- Top secret facts: If your errors end up being displayed to a user and they can manipulate the input, they can already guess and check what files are on your system. If you introduce this library, an attacker could gain more information (such as specific file permissions) and make it easier to list directory contents. We recommend using this library in contexts where developer logs are kept separate from user-facing errors or where the user already has access to the entire disk (such as in a Cloud Native Buildpack).
- Stranger than fiction: Facts provided by this library make an effort to be as correct as possible but might provide incomplete or conflicting information. What does that mean? The best way to know if you can do something on disk is to try it and see if it succeeds. If your system uses a custom file permission restriction system, it might show a path with `read` permission without read access. Consider the information provided by path facts as a good starting point on where to focus your investigation rather than as immutable and indisputable truth.

## Why not path recommendations instead of path facts?

Initial efforts on this library were geared towards giving concrete recommendations such as telling people to `mkdir -p <directory>` or running a specific `chmod` command. That could still be a worthwhile effort, but that requires that we know both the facts on disk as well as the intent of the programmer.

To explain: Suggestions and recommendations are better the more true they are. If an error message says "please try again" and trying again doesn't fix it, but the message continues to assert "please try again," it's not so much a valid suggestion but more wishful thinking on the error message author. If a path doesn't exist, that's usually bad, so we suggest you create it via the `touch` command. Except if you're trying to create a file, then that path not existing is good, and suggesting that you create it would introduce an error where none existed before. So, any suggestions must be context-aware and task-aware.

Complicating things further: implementation details matter when determining the disk's state **should** be. For example, Rust's [std::fs::rename](https://doc.rust-lang.org/std/fs/fn.rename.html) function will error if the "to" path exists unless it's on Unix and it's a directory that is empty and the "from" path is also a directory. But on Windows, the "to" path cannot be a directory. That's a lot of caveats to consider!

Effectively, the only way to deliver truly accurate recommendations for some operations would be to reverse engineer them. That's fine in moderation. We do that a little here, traversing directories in a parent chain to see which one doesn't exist. However, if your implementation is overly coupled to internally described logic, it can be difficult to maintain it if the reference implementation changes without warning. Then suddenly, previously valid suggestions are no longer true!

There are some ways around this inside-out implementation-coupling problem. For example, [synax_suggest](https://github.com/ruby/syntax_suggest) tries to know as little as possible about Ruby grammar and parsing. It tells the user things that are true, such as "if you take these lines of code together, they're invalid Ruby." Then it presents that truthy information in a way the user can consume and actualize. It's not always a perfect result, but it's correct enough to be helpful more often than it's harmful. A more scholarly way to frame this would be looking at it through the lens of [soundeness versus completeness and precision](https://cacm.acm.org/blogcacm/soundness-and-completeness-defined-with-precision/). We aim for "more precise" i.e., "it reports fewer non-errors." It also means we could possibly stop at some point earlier than "completely reverse-engineer rust stdlib behavior" and somewhere further than "simply state the facts". However, we're here. We have the facts, we might as well show those.

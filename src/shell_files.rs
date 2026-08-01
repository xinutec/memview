//! Which files a shell command used, and which way.
//!
//! [`crate::shell`] reads the syntax; this reads the *meaning*, and it is the
//! half no parser crate supplies. `cat x` opens a file, `grep x y` opens the
//! second word and not the first, `cp a b` writes only the last — none of that
//! is visible in the grammar, and all of it decides whether a path belongs to an
//! agent's record.
//!
//! **Conservative by construction.** A command not in the table below
//! contributes nothing at all and is counted in an unhandled report, which is
//! the worklist for extending it — the same loop the grammar was grown by. The
//! rule throughout is undercount rather than invent: a missing path costs one
//! agent one point, an invented one puts a file in a record that never had it,
//! and only the second is a lie.
//!
//! **Nothing is expanded and nothing is looked up on disk.** `$VAR` makes a path
//! unusable rather than guessed at, and a glob is recorded exactly as written —
//! resolving `plan/*.dhall` against today's checkout would attribute work to
//! files that did not exist then and miss every file since deleted. `~` and
//! `$HOME` are the sole exception, because they have one knowable value.

use std::collections::BTreeMap;

use crate::shell::Simple;

/// A file a command used, absolute, and which way it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUse {
    pub path: String,
    /// `true` when the command changed the file: a `>` redirect, `sed -i`, the
    /// destination of a `cp`, an `rm`. Reads and writes stay apart for the same
    /// reason they do in the tool-call miner — consulting a file and being
    /// responsible for it are different claims.
    pub write: bool,
}

/// What one script's worth of commands used, with the misses on the record.
#[derive(Debug, Default)]
pub struct Extract {
    pub files: Vec<FileUse>,
    /// Commands whose operands were not interpreted, by name. Not an error —
    /// the honest size of what this does not yet read.
    pub unhandled: BTreeMap<String, usize>,
    /// Commands the table did interpret, whether or not they named a file.
    pub handled: usize,
}

/// Which operands of a command are files, and which way they go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Every operand is read: `cat a b`, `wc -l x`.
    AllRead,
    /// Every operand is changed: `rm a`, `touch b`, `tee c`.
    AllWrite,
    /// The first operand is a pattern or a program rather than a file, and the
    /// rest are read: `grep pat f`, `awk 'prog' f`, `jq . f`.
    SkipFirstRead,
    /// As above, but the files are rewritten in place: `sed -i 's/…/…/' f`.
    SkipFirstWrite,
    /// Only the first operand, read — the script an interpreter runs. What
    /// follows belongs to the script, not to the shell, and is not ours to read.
    FirstRead,
    /// Sources read, last operand written: `cp a b`, `mv a b`, `install a b`.
    LastWritten,
    /// Understood, and names no file: `echo`, `sleep`, `ssh`, the loop
    /// keywords. Distinct from absent, which means *not read yet* — without
    /// this the worklist is headed by `echo` forever, and 58,243 commands that
    /// were never going to name a file look like a gap in coverage.
    None,
}

/// The commands whose file use is unambiguous, and the flags that take a value.
///
/// The valued-flag lists are not decoration. Without them `grep -A 3 pat f`
/// offers `3` as the pattern and `pat` as a file — a word that is not a path
/// being recorded as one, which is the failure mode this whole module is shaped
/// to avoid.
fn spec(name: &str, argv: &[String]) -> Option<(Kind, &'static [&'static str])> {
    const CONTEXT: &[&str] = &[
        "-e", "--regexp", "-f", "--file", "-m", "-A", "-B", "-C", "-d",
    ];
    Some(match name {
        // Read the whole of every operand.
        "cat" | "bat" | "head" | "tail" | "less" | "more" | "wc" | "nl" | "od" | "xxd"
        | "hexdump" | "strings" | "file" | "stat" | "du" | "md5sum" | "sha1sum" | "sha256sum"
        | "shasum" | "cksum" | "sort" | "uniq" | "cut" | "column" | "base64" | "diff" | "cmp"
        | "comm" | "ls" | "tree" | "readlink" | "realpath" | "wsl" => (Kind::AllRead, &["-n"]),
        // A pattern first, then the files.
        "grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack" => (Kind::SkipFirstRead, CONTEXT),
        "awk" | "gawk" | "jq" | "yq" => (Kind::SkipFirstRead, &["-F", "-v", "--arg", "--argjson"]),
        // `sed` rewrites in place only with `-i`, and `-i.bak`/`-i''` are the
        // same flag: matched by prefix, not equality.
        "sed" => {
            let inplace = argv
                .iter()
                .any(|a| a.starts_with("-i") && !a.starts_with("--"));
            let kind = if inplace {
                Kind::SkipFirstWrite
            } else {
                Kind::SkipFirstRead
            };
            (kind, &["-e", "-f", "--expression"])
        }
        // Changed, created or destroyed.
        "rm" | "touch" | "truncate" | "unlink" | "shred" | "chmod" | "chown" | "chgrp" => {
            (Kind::AllWrite, &["--reference"])
        }
        "tee" => (Kind::AllWrite, &[]),
        // Sources first, destination last.
        "cp" | "mv" | "install" | "ln" | "rsync" | "scp" => (Kind::LastWritten, &["--exclude"]),
        // An interpreter's first operand is the script it runs. The flags listed
        // carry code or a module name, never a path.
        "python" | "python3" => (Kind::FirstRead, &["-c", "-m", "-W"]),
        "node" | "deno" | "bun" => (Kind::FirstRead, &["-e", "-p", "--eval"]),
        "bash" | "sh" | "zsh" | "dash" | "ksh" => (Kind::FirstRead, &["-c", "-o"]),
        "ruby" | "perl" => (Kind::FirstRead, &["-e", "-E", "-I"]),
        "source" | "." => (Kind::FirstRead, &[]),
        // `find .` and `du -sh dir`: the first operand is where it looked, and
        // everything after is an expression full of patterns that are not paths.
        "find" | "fd" => (
            Kind::FirstRead,
            &["-name", "-iname", "-path", "-type", "-exec"],
        ),
        // A database is a file, and the query after it is not.
        "sqlite3" => (Kind::FirstRead, &[]),

        // ---- understood, and naming no file ----
        //
        // Output, timing, process and shell builtins. They can still carry a
        // redirect, which is collected before any of this.
        "echo" | "printf" | "true" | "false" | ":" | "sleep" | "pwd" | "date" | "seq" | "yes"
        | "clear" | "tr" | "rev" | "basename" | "dirname" | "sync" | "eval" | "read" | "set"
        | "unset" | "export" | "alias" | "shift" | "local" | "exit" | "trap" | "wait" | "jobs"
        | "disown" | "hash" | "type" | "which" | "whoami" | "id" | "hostname" | "uname"
        | "sw_vers" | "df" | "uptime" | "open" => (Kind::None, &[]),
        // Process control: the operands are pids and patterns.
        "kill" | "pkill" | "pgrep" | "killall" | "ps" | "lsof" | "top" | "nproc" => {
            (Kind::None, &[])
        }
        // Directories, not files. Creating one is work, but a directory is not
        // a thing the index can attribute, and `mkdir -p` names several at once.
        "mkdir" | "rmdir" | "pushd" | "popd" => (Kind::None, &[]),
        // Another machine's filesystem, whatever the operands look like.
        "ssh" | "sftp" | "kubectl" | "docker" | "podman" | "helm" | "systemctl" | "launchctl"
        | "adb" | "xcrun" | "curl" | "wget" | "gh" | "aws" | "rclone" | "restic" | "borg" => {
            (Kind::None, &[])
        }
        // Build and package tooling: it reads whole trees by convention rather
        // than by argument, so its operands are targets and subcommands, never
        // paths. Attributing a repository to whoever ran `cargo test` in it
        // would make every session an owner of everything it built.
        "cargo" | "rustc" | "npm" | "pnpm" | "yarn" | "npx" | "make" | "cmake" | "gradle"
        | "gradlew" | "mvn" | "pip" | "pip3" | "uv" | "poetry" | "go" | "ng" | "tsc" | "nix"
        | "nix-shell" | "nix-build" | "nix-env" | "nixos-rebuild" | "direnv" | "brew"
        | "flutter" | "dart" | "swift" | "javac" | "kotlinc" => (Kind::None, &[]),
        // Loop and conditional keywords, which the grammar leaves as ordinary
        // words on purpose (`echo done` must not end a loop). None of them can
        // name a file, so none can put a wrong path in the index.
        "for" | "done" | "fi" | "esac" | "case" | "in" | "break" | "continue" | "return"
        | "select" | "function" | "[" | "[[" | "test" => (Kind::None, &[]),
        _ => return None,
    })
}

/// Commands that run another command, and contribute nothing themselves.
///
/// Stripped so the real command is the one looked up: `sudo rm x` is an `rm`,
/// and without this it is an unhandled `sudo`. `xargs` is here too — the command
/// after it is real, even though its operands arrive on stdin and so cannot be
/// read from the text at all.
fn wrapper(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "sudo" => &["-u", "-g"],
        "env" => &["-C", "-u"],
        "timeout" | "nice" | "ionice" => &[],
        "nohup" | "setsid" | "time" | "command" | "exec" | "stdbuf" | "builtin" => &[],
        "xargs" => &["-I", "-n", "-P", "-d", "-a"],
        // A keyword standing in front of a command, which the grammar hands
        // over as an ordinary first word. Stripping them is what makes a loop
        // body readable at all: `for f in *; do cat "$f"; done` arrives as a
        // command named `do`, and the `cat` behind it would be lost.
        "do" | "then" | "else" | "elif" | "if" | "while" | "until" | "!" => &[],
        _ => return None,
    })
}

/// Files named by a `git` invocation, or `None` when it names none.
///
/// Git needs its own rule for two reasons the table cannot express. `-C <dir>`
/// moves the directory the operands resolve against, and most subcommands take
/// *revisions* where a path would go — `git diff origin/main` would otherwise
/// record a file called `origin/main`, which is precisely the invented path this
/// module refuses to produce.
///
/// So only the unambiguous cases count: the subcommands whose operands are
/// always paths, and anything after the `--` that exists to say "paths follow".
fn git_files<'a>(
    argv: &'a [String],
    base: &mut Option<String>,
    home: &str,
) -> Vec<(&'a str, bool)> {
    let mut rest = argv.iter().skip(1);
    let mut sub = None;
    // Global flags come before the subcommand, and `-C` among them decides where
    // everything after is resolved.
    while let Some(word) = rest.next() {
        match word.as_str() {
            "-C" => {
                if let Some(dir) = rest.next() {
                    *base = resolve(dir, base.as_deref(), home);
                }
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                rest.next();
            }
            flag if flag.starts_with('-') => {}
            other => {
                sub = Some(other);
                break;
            }
        }
    }
    let Some(sub) = sub else {
        return Vec::new();
    };
    // Operands of the subcommand, with its flags dropped.
    let words: Vec<&str> = rest
        .map(String::as_str)
        .skip_while(|w| w.starts_with('-') && *w != "--")
        .collect();
    let after_sep = words.iter().position(|w| *w == "--");
    match (sub, after_sep) {
        // Staging, deleting or restoring a path is a claim on it as strong as an
        // edit — the file is being put into, or taken out of, the tree.
        // `checkout` is deliberately absent: it takes a branch as often as a
        // path, and `git checkout origin/main` would file a write against a file
        // of that name. It is readable only in its `--` form, below.
        ("add" | "rm" | "restore" | "stage" | "mv", _) => words
            .iter()
            .filter(|w| **w != "--")
            .map(|w| (*w, true))
            .collect(),
        // `git log -- src/x.rs`, `git diff -- frontend/`: the separator is the
        // author saying these are paths, which is exactly the guarantee needed.
        (_, Some(at)) => words[at + 1..].iter().map(|w| (*w, false)).collect(),
        _ => Vec::new(),
    }
}

/// Filenames worth recognising without a `/` or an extension.
///
/// The shape test below wants a slash or a suffix before it will believe a word
/// is a path, which is what keeps a stray flag value out of the index. These are
/// the everyday exceptions to it.
const BARE_FILENAMES: &[&str] = &[
    "Makefile",
    "Dockerfile",
    "Justfile",
    "Rakefile",
    "Gemfile",
    "Vagrantfile",
    "Procfile",
    "README",
    "LICENSE",
    "COPYING",
    "CHANGELOG",
    "NOTICE",
    "AUTHORS",
];

/// Whether a word is shaped like a path at all.
///
/// **The guard that keeps invented paths out.** Every operand that survives the
/// flag rules is a candidate, and some of them are not files — a stray context
/// number, a `git` refspec, the word after a flag this table does not know
/// takes a value. Requiring a slash, a `~`, or an extension throws those away.
///
/// It costs something real and known: `rg foo src` loses `src`, because a bare
/// directory name is indistinguishable from a bare non-path. That is the side
/// of the trade the rule asks for — a lost read is an undercount, a kept
/// non-path is a fabrication.
fn looks_like_path(word: &str) -> bool {
    if word.starts_with('~') || word.contains('/') {
        return true;
    }
    if BARE_FILENAMES.contains(&word) {
        return true;
    }
    // An extension: a dot inside the word with a short alphanumeric tail.
    match word.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// Turn a word into an absolute path, or refuse it.
///
/// Refuses more than it accepts, and each refusal is a category that would
/// otherwise put a wrong path in the index:
/// - an unexpanded `$VAR` — its value is not knowable now, if ever;
/// - `host:path` and anything with a scheme — another machine's filesystem, or
///   a URL, neither of which is a file here;
/// - anything at all when the working directory is unknown, since a relative
///   path without one names nothing.
fn resolve(word: &str, cwd: Option<&str>, home: &str) -> Option<String> {
    if word.is_empty() || word == "-" || word.contains("://") {
        return None;
    }
    // `/dev/null` and its siblings are plumbing, not files. Left in, the single
    // busiest path in the whole corpus is `/dev/null` with 25,407 writes, which
    // says nothing about anyone's work.
    if word.starts_with("/dev/") {
        return None;
    }
    // `~` and `$HOME` are the one expansion with a knowable value.
    let expanded = if let Some(rest) = word.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if word == "~" {
        home.to_string()
    } else if let Some(rest) = word
        .strip_prefix("$HOME/")
        .or_else(|| word.strip_prefix("${HOME}/"))
    {
        format!("{home}/{rest}")
    } else {
        word.to_string()
    };
    if expanded.contains('$') {
        return None;
    }
    // `isis:/var/log`, `user@host:path` — a remote path, which must not enter a
    // local index. A leading `/` or `.` cannot be a host, and a Windows drive
    // letter is not something this corpus contains.
    if let Some((head, _)) = expanded.split_once(':')
        && !head.contains('/')
    {
        return None;
    }
    let absolute = if expanded.starts_with('/') {
        expanded
    } else {
        format!("{}/{}", cwd?.trim_end_matches('/'), expanded)
    };
    Some(normalise(&absolute))
}

/// Resolve `.` and `..` textually, without touching the filesystem.
///
/// Textual on purpose: the path may name a file that no longer exists, and
/// asking the disk about it would answer for today's checkout rather than for
/// the day the command ran.
fn normalise(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The operands of a command: its words with the program and its flags removed.
///
/// `--` ends the flags, `--flag=value` carries its own value, and a flag named
/// in `valued` eats the word after it.
fn operands<'a>(argv: &'a [String], valued: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = argv.iter().skip(1);
    let mut flags = true;
    while let Some(word) = rest.next() {
        if flags && word == "--" {
            flags = false;
        } else if flags && word.starts_with('-') && word.len() > 1 {
            if valued.contains(&word.as_str()) {
                rest.next();
            }
        } else if !word.is_empty() {
            // An empty word is never a file, and never a pattern either. Kept,
            // it takes the place of one: BSD `sed -i '' 's/a/b/' f` offers `''`
            // as the script to skip, and the *real* script `s/a/b/` is then
            // recorded as a path, since it is full of slashes.
            out.push(word.as_str());
        }
    }
    out
}

/// Strip leading `VAR=value` assignments and any wrapper commands, leaving the
/// command that actually ran.
///
/// `REV=$(git rev-parse HEAD) nohup ./deploy.sh` is a `./deploy.sh`, and the
/// parser hands the assignment over as an ordinary word.
fn unwrap_command(argv: &[String]) -> &[String] {
    let mut argv = argv;
    loop {
        let Some(head) = argv.first() else {
            return argv;
        };
        // An assignment, not a command: `FOO=bar`, and never a path or a flag.
        if let Some((name, _)) = head.split_once('=')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            argv = &argv[1..];
            continue;
        }
        let Some(valued) = wrapper(basename(head)) else {
            return argv;
        };
        // Skip the wrapper's own flags and operands-that-are-not-commands
        // (`timeout 30`, `nice -n 5`) until a word that could be a command.
        let mut i = 1;
        while i < argv.len() {
            let word = &argv[i];
            let name = basename(head);
            // A wrapper's own operands, which are not the command: `timeout 30`
            // takes a duration, `env FOO=bar` takes assignments.
            let own = (name == "timeout" && word.chars().all(|c| c.is_ascii_digit()))
                || (name == "env" && word.contains('='));
            if word.starts_with('-') && word.len() > 1 {
                i += if valued.contains(&word.as_str()) {
                    2
                } else {
                    1
                };
            } else if own {
                i += 1;
            } else {
                break;
            }
        }
        if i >= argv.len() {
            return &argv[argv.len()..];
        }
        argv = &argv[i..];
    }
}

/// A command's name, without the path it was invoked by: `./scripts/verify.sh`
/// and `/usr/bin/sed` name `verify.sh` and `sed`.
fn basename(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// Every file the commands of one script used, resolved against `cwd`.
///
/// `cwd` is the directory the `Bash` call ran in — the transcripts record it on
/// every line, and it is the one piece of context a relative path cannot be read
/// without. `None` where it is unknown, in which case only absolute paths
/// survive.
pub fn extract(cmds: &[Simple], cwd: Option<&str>, home: &str) -> Extract {
    let mut out = Extract::default();
    // Working directory per subshell scope. A `cd` inside `( … )` writes an
    // entry only that scope and its children can see, so the script it returns
    // to is unaffected — which is what the shell does.
    let mut dirs: BTreeMap<Vec<usize>, Option<String>> = BTreeMap::new();
    dirs.insert(Vec::new(), cwd.map(str::to_string));

    for cmd in cmds {
        let here = current(&dirs, &cmd.scope);
        // A redirect names a file whatever the command is, so it is collected
        // before the table is consulted and counts even for a command the table
        // does not know: `./gradlew build > /tmp/out` is still a write.
        for redirect in &cmd.redirects {
            if looks_like_path(&redirect.target)
                && let Some(path) = resolve(&redirect.target, here.as_deref(), home)
            {
                out.files.push(FileUse {
                    path,
                    write: redirect.write,
                });
            }
        }

        let argv = unwrap_command(&cmd.argv);
        let Some(head) = argv.first() else {
            continue;
        };
        let name = basename(head);

        if name == "cd" {
            // The scope's own directory, not its parent's: `(cd x && …)` moves
            // the subshell alone. An unresolvable target makes it *unknown*
            // rather than unchanged — carrying on with the old one would resolve
            // every later relative path against the wrong directory.
            let target = operands(argv, &[]).first().map(|w| w.to_string());
            let moved = match target {
                Some(word) => resolve(&word, here.as_deref(), home),
                None => Some(home.to_string()),
            };
            dirs.insert(cmd.scope.clone(), moved);
            out.handled += 1;
            continue;
        }

        if name == "git" {
            out.handled += 1;
            // `-C` moves only this command, so the scope's directory is left
            // alone: `git -C ~/Code/other log` does not move the script.
            let mut base = here.clone();
            for (word, write) in git_files(argv, &mut base, home) {
                if looks_like_path(word)
                    && let Some(path) = resolve(word, base.as_deref(), home)
                {
                    out.files.push(FileUse { path, write });
                }
            }
            continue;
        }

        // A command invoked by path — `./scripts/verify.sh`, `../gradlew` — is a
        // file that was used, whatever else the command turns out to mean. The
        // table below classifies *operands*; this is about the program itself,
        // so it applies either way.
        if head.contains('/')
            && let Some(path) = resolve(head, here.as_deref(), home)
        {
            out.files.push(FileUse { path, write: false });
        }

        let Some((kind, valued)) = spec(name, argv) else {
            if head.contains('/') {
                // Readable after all: what it did is unknown, but that it ran is
                // not, so this is not a gap in the table.
                out.handled += 1;
            } else {
                *out.unhandled.entry(name.to_string()).or_insert(0) += 1;
            }
            continue;
        };
        out.handled += 1;

        let words = operands(argv, valued);
        let chosen: Vec<(&str, bool)> = match kind {
            Kind::AllRead => words.iter().map(|w| (*w, false)).collect(),
            Kind::AllWrite => words.iter().map(|w| (*w, true)).collect(),
            Kind::SkipFirstRead => words.iter().skip(1).map(|w| (*w, false)).collect(),
            Kind::SkipFirstWrite => words.iter().skip(1).map(|w| (*w, true)).collect(),
            Kind::FirstRead => words.first().map(|w| (*w, false)).into_iter().collect(),
            // With one operand there is no source, only a destination — `cp x`
            // is not a command anyone ran, but `rsync -a src/` reaches here.
            Kind::LastWritten => words
                .iter()
                .enumerate()
                .map(|(i, w)| (*w, i + 1 == words.len()))
                .collect(),
            Kind::None => Vec::new(),
        };
        for (word, write) in chosen {
            if looks_like_path(word)
                && let Some(path) = resolve(word, here.as_deref(), home)
            {
                out.files.push(FileUse { path, write });
            }
        }
    }
    out
}

/// The working directory in force for a scope: its own if it has moved, else
/// the nearest enclosing one that has.
fn current(dirs: &BTreeMap<Vec<usize>, Option<String>>, scope: &[usize]) -> Option<String> {
    (0..=scope.len())
        .rev()
        .find_map(|n| dirs.get(&scope[..n].to_vec()))
        .cloned()
        .flatten()
}

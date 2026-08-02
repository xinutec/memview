//! What a shell command *does*, as a type rather than a table lookup.
//!
//! [`crate::shell`] gives words; this gives meaning. The first version went
//! straight from words to "which paths, read or written", and that projection
//! threw away everything else the command said: `grep hsmmDecode src/x.ts`
//! became one read, indistinguishable from `cat src/x.ts`, though only one of
//! them records *what was being looked for*.
//!
//! Naming the operation keeps that. A [`Op::Search`] knows its pattern, a
//! [`Op::Move`] knows a file's old name as well as its new one, and a
//! [`Op::Run`] knows which script was executed. The file use that
//! [`crate::shell_files`] mines is then a *projection* of these — one obvious
//! function — rather than a second table that has to be kept in step with this
//! one.
//!
//! Everything the older table refused, this refuses identically: an unknown
//! command is [`Op::Unknown`] and contributes nothing, nothing is expanded
//! beyond `~`/`$HOME`, nothing is looked up on disk, and a word must be shaped
//! like a path before it can be one.

/// What one command does.
///
/// The variants are the operations this corpus actually performs — the same
/// closed set the extraction table described, now stated once and in a form
/// that can be asked questions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Reads whole files: `cat`, `head`, `wc`, `ls`, an interpreter's `<`.
    Read { paths: Vec<String> },
    /// Creates or replaces: `tee`, `touch`, `truncate`.
    Write { paths: Vec<String> },
    /// Deletes.
    Remove { paths: Vec<String>, recursive: bool },
    /// Copies. The destination is written; the sources are read.
    Copy { from: Vec<String>, to: String },
    /// Renames — the one operation that knows a file had another name, which no
    /// count of reads and writes can express.
    Move { from: Vec<String>, to: String },
    /// Searches for something. **The pattern is the point**: it is what the
    /// agent was looking for, and the older projection discarded it.
    Search { pattern: String, paths: Vec<String> },
    /// Rewrites text through a program — `sed`, `awk`, `jq`. In place or not,
    /// which is the whole difference between reading a file and changing it.
    Transform {
        program: String,
        /// A program given as a file (`sed -f fix.sed`), which is *read* even
        /// when the operands are being rewritten — so it cannot ride along in
        /// `paths`, whose direction `in_place` decides.
        program_file: Option<String>,
        paths: Vec<String>,
        in_place: bool,
    },
    /// Runs a script: an interpreter with a file, or a program invoked by path.
    Run { script: String },
    /// Changes the working directory. `None` is `cd` with no argument (home),
    /// and an unresolvable target is [`Op::Unknown`] rather than a guess.
    ChangeDir { to: Option<String> },
    /// A git subcommand worth naming. Staging is deliberately absent from the
    /// file projection — see [`GitOp`].
    Git(GitOp),
    /// Understood, and does nothing with files: `echo`, `sleep`, `ssh`, the
    /// loop keywords. Distinct from `Unknown`, which means *not read yet*.
    Nothing,
    /// Not in the table. Carries its name so the gap can be counted and worked
    /// down rather than silently ignored.
    Unknown { name: String },
}

/// The git subcommands whose effect on files is unambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOp {
    /// `git add` — **staging, which changes no file.** Named rather than
    /// dropped: it was 37% of every shell-derived write when it was miscounted
    /// as one, and a variant that exists but projects to nothing is the way to
    /// keep that decision visible instead of re-making it.
    Stage { paths: Vec<String> },
    /// `git rm`, `git restore`, `git mv` — these do change the working tree.
    Alter { paths: Vec<String> },
    /// Paths after a `--`, which the author has declared to be paths.
    Inspect { paths: Vec<String> },
    /// Everything else: `status`, `log`, `commit`, `push`.
    Other { subcommand: String },
}

/// Filenames worth recognising without a `/` or an extension.
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
/// **The guard that keeps invented paths out.** Some operands are not files — a
/// stray context number, a git refspec, the word after a flag whose
/// value-taking this table does not know about. Requiring a slash, a `~`, or an
/// extension throws those away.
///
/// It costs something real and known: `rg foo src` loses `src`, because a bare
/// directory name is indistinguishable from a bare non-path. That is the side of
/// the trade to be on — a lost read is an undercount, a kept non-path is a
/// fabrication.
pub fn looks_like_path(word: &str) -> bool {
    if word.starts_with('~') || word.contains('/') {
        return true;
    }
    if BARE_FILENAMES.contains(&word) {
        return true;
    }
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
/// - `host:path` and anything with a scheme — another machine, or a URL;
/// - `/dev/*`, which is plumbing: left in, `/dev/null` is the busiest path in
///   the whole corpus at 25,407 writes and says nothing about anyone's work;
/// - anything at all when the working directory is unknown, since a relative
///   path without one names nothing.
pub fn resolve(word: &str, cwd: Option<&str>, home: &str) -> Option<String> {
    if word.is_empty() || word == "-" || word.contains("://") || word.starts_with("/dev/") {
        return None;
    }
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
    // local index. A leading `/` or `.` cannot be a host.
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

/// Resolve `.` and `..` textually, without touching the filesystem — the path
/// may name a file that no longer exists, and the disk would answer for today's
/// checkout rather than for the day the command ran.
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

/// The paths among these words, resolved and in order.
fn paths(words: &[&str], cwd: Option<&str>, home: &str) -> Vec<String> {
    words
        .iter()
        .filter(|w| looks_like_path(w))
        .filter_map(|w| resolve(w, cwd, home))
        .collect()
}

/// The operands of a command: its words with the program and its flags removed.
///
/// `--` ends the flags, `--flag=value` carries its own value, and a flag named
/// in `valued` eats the word after it. An empty word is dropped — kept, BSD
/// `sed -i '' 's/a/b/' f` offers `''` as the script to skip and the real script
/// is then recorded as a path, since it is full of slashes.
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
            out.push(word.as_str());
        }
    }
    out
}

/// Whether any of `flags` appears in `argv`, in either form.
fn has_flag(argv: &[String], flags: &[&str]) -> bool {
    flags.iter().any(|flag| {
        argv.iter()
            .any(|word| word == flag || word.starts_with(&format!("{flag}=")))
    })
}

/// The values given to any of `flags`, in order.
fn flag_values<'a>(argv: &'a [String], flags: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = argv.iter().skip(1);
    while let Some(word) = rest.next() {
        if flags.contains(&word.as_str())
            && let Some(value) = rest.next()
        {
            out.push(value.as_str());
        } else if let Some((flag, value)) = word.split_once('=')
            && flags.contains(&flag)
        {
            out.push(value);
        }
    }
    out
}

/// A command's name without the path it was invoked by: `./scripts/verify.sh`
/// and `/usr/bin/sed` name `verify.sh` and `sed`.
pub fn basename(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// Commands that run another command and contribute nothing themselves, with
/// the flags of their own that take a value.
///
/// Stripped so the real command is the one classified: `sudo rm x` is an `rm`.
/// The keywords are here for the same reason — the grammar leaves `do` as an
/// ordinary first word, and `for f in *; do cat "$f"; done` would otherwise be a
/// command named `do` with the `cat` behind it lost.
fn wrapper(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "sudo" => &["-u", "-g"],
        "env" => &["-C", "-u"],
        "timeout" | "nice" | "ionice" => &[],
        "nohup" | "setsid" | "time" | "command" | "exec" | "stdbuf" | "builtin" => &[],
        "xargs" => &["-I", "-n", "-P", "-d", "-a"],
        "do" | "then" | "else" | "elif" | "if" | "while" | "until" | "!" => &[],
        _ => return None,
    })
}

/// Strip leading `VAR=value` assignments and any wrappers, leaving the command
/// that actually ran.
pub fn unwrap_command(argv: &[String]) -> &[String] {
    let mut argv = argv;
    loop {
        let Some(head) = argv.first() else {
            return argv;
        };
        if let Some((name, _)) = head.split_once('=')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            argv = &argv[1..];
            continue;
        }
        let name = basename(head);
        let Some(valued) = wrapper(name) else {
            return argv;
        };
        let mut i = 1;
        while i < argv.len() {
            let word = &argv[i];
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

/// Context-taking flags for the search commands.
///
/// Not decoration: without `-A` here, `grep -A 3 dhall f` offers `3` as the
/// pattern and `dhall` as a file — a word that is not a path, resolved against
/// the working directory and recorded as one./// Context-taking flags for the search commands.
///
/// Not decoration: without `-A` here, `grep -A 3 dhall f` offers `3` as the
/// pattern and `dhall` as a file — a word that is not a path, resolved against
/// the working directory and recorded as one.
const SEARCH_FLAGS: &[&str] = &[
    "-e", "--regexp", "-f", "--file", "-m", "-A", "-B", "-C", "-d", "-g", "--glob", "-t", "--type",
];

/// Flags of one command, as the classifier needs to know them.
///
/// `script` supplies the pattern or program, so that no *operand* does — and
/// that distinction is load-bearing: `sed 's/a/b/' f` and `sed -e 's/a/b/' f`
/// take the same two things in a different order, so skipping a script operand
/// that is not there eats the file.
#[derive(Debug, Clone, Copy)]
struct Flags {
    /// Flags that consume the following word, which is therefore not an operand.
    valued: &'static [&'static str],
    /// Flags that supply the pattern or program.
    script: &'static [&'static str],
    /// Those among them naming a *file* of patterns, itself a read.
    script_file: &'static [&'static str],
}

impl Flags {
    const NONE: Flags = Flags {
        valued: &[],
        script: &[],
        script_file: &[],
    };
    const fn valued(valued: &'static [&'static str]) -> Flags {
        Flags {
            valued,
            script: &[],
            script_file: &[],
        }
    }
}

/// What kind of command this is — **the closed set, named once**.
///
/// The classifier below matches on this rather than on the command name, so the
/// string is parsed exactly once, at [`verb`], and a name nobody taught it
/// cannot slip through a `_ =>` arm pretending to be understood. Grouped by
/// *behaviour* rather than one variant per command: `cat` and `wc` differ in
/// nothing this cares about.
#[derive(Debug, Clone, Copy)]
enum Verb {
    /// Reads every operand: `cat`, `head`, `wc`, `ls`.
    Read,
    /// A pattern, then the files it was looked for in.
    Search(Flags),
    /// A program applied to files, rewriting them only with `-i`.
    Stream { flags: Flags, honours_i: bool },
    /// Deletes every operand.
    Remove,
    /// Creates or replaces every operand.
    Overwrite,
    /// Sources read, destination written.
    Copy(Flags),
    /// The same, but the destination *is* the source under a new name.
    Move(Flags),
    /// Runs its first operand, which is a script.
    Interpreter(Flags),
    /// Looked in its first operand; everything after is an expression.
    Walk(Flags),
    /// Moves the working directory.
    ChangeDir,
    /// Needs its own reading — revisions sit where paths would.
    Git,
    /// Understood, and does nothing with files.
    ///
    /// Distinct from a name that is absent, and the distinction matters: without
    /// it the worklist of commands still to support is headed by `echo` forever,
    /// and 58,243 commands that were never going to name a file look like a gap
    /// in coverage.
    NoFiles,
}

/// The one place a command name is read. `None` means "not taught yet", which is
/// [`Op::Unknown`] — never a silent success.
fn verb(name: &str) -> Option<Verb> {
    const SEARCH: Flags = Flags {
        valued: SEARCH_FLAGS,
        script: &["-e", "--regexp", "-f", "--file"],
        script_file: &["-f", "--file"],
    };
    Some(match name {
        "cat" | "bat" | "head" | "tail" | "less" | "more" | "wc" | "nl" | "od" | "xxd"
        | "hexdump" | "strings" | "file" | "stat" | "du" | "md5sum" | "sha1sum" | "sha256sum"
        | "shasum" | "cksum" | "sort" | "uniq" | "cut" | "column" | "base64" | "diff" | "cmp"
        | "comm" | "ls" | "tree" | "readlink" | "realpath" => Verb::Read,

        "grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack" => Verb::Search(SEARCH),

        // `-i.bak` and `-i''` are the same flag wearing a suffix, which is why
        // `honours_i` is a property of the command and the match is by prefix.
        "sed" => Verb::Stream {
            flags: Flags {
                valued: &["-e", "-f", "--expression"],
                script: &["-e", "--expression", "-f", "--file"],
                script_file: &["-f", "--file"],
            },
            honours_i: true,
        },
        "awk" | "gawk" => Verb::Stream {
            flags: Flags {
                valued: &["-F", "-v", "-f", "--file"],
                script: &["-f", "--file"],
                script_file: &["-f", "--file"],
            },
            honours_i: false,
        },
        "jq" | "yq" => Verb::Stream {
            flags: Flags {
                valued: &["--arg", "--argjson", "-f", "--from-file"],
                script: &["-f", "--from-file"],
                script_file: &["-f", "--from-file"],
            },
            honours_i: false,
        },

        "rm" | "shred" | "unlink" => Verb::Remove,
        "touch" | "truncate" | "tee" | "chmod" | "chown" | "chgrp" => Verb::Overwrite,
        // `--exclude` takes a *pattern*, and a pattern shaped like a path
        // (`dist/`) would otherwise be recorded as a file that was copied.
        "cp" | "install" | "ln" | "rsync" | "scp" => {
            Verb::Copy(Flags::valued(&["--exclude", "--include", "--filter"]))
        }
        "mv" => Verb::Move(Flags::NONE),

        // An interpreter's flags carry code or a module name, never a path.
        "python" | "python3" => Verb::Interpreter(Flags::valued(&["-c", "-m", "-W"])),
        "node" | "deno" | "bun" => Verb::Interpreter(Flags::valued(&["-e", "-p", "--eval"])),
        "bash" | "sh" | "zsh" | "dash" | "ksh" => Verb::Interpreter(Flags::valued(&["-c", "-o"])),
        "ruby" | "perl" => Verb::Interpreter(Flags::valued(&["-e", "-E", "-I"])),
        "source" | "." | "sqlite3" => Verb::Interpreter(Flags::NONE),

        "find" | "fd" => Verb::Walk(Flags::valued(&[
            "-name", "-iname", "-path", "-type", "-exec",
        ])),
        "cd" => Verb::ChangeDir,
        "git" => Verb::Git,

        // Output, timing and shell builtins. They can still carry a redirect,
        // which is collected separately and counts either way.
        "echo" | "printf" | "true" | "false" | ":" | "sleep" | "pwd" | "date" | "seq" | "yes"
        | "clear" | "tr" | "rev" | "basename" | "dirname" | "sync" | "eval" | "read" | "set"
        | "unset" | "export" | "alias" | "shift" | "local" | "exit" | "trap" | "wait" | "jobs"
        | "disown" | "hash" | "type" | "which" | "whoami" | "id" | "hostname" | "uname"
        | "sw_vers" | "df" | "uptime" | "open" | "wsl"
        // Process control: the operands are pids and patterns.
        | "kill" | "pkill" | "pgrep" | "killall" | "ps" | "lsof" | "top" | "nproc"
        // Directories, not files. Creating one is work, but a directory is not a
        // thing the index can attribute, and `mkdir -p` names several at once.
        | "mkdir" | "rmdir" | "pushd" | "popd"
        // Another machine's filesystem, whatever the operands look like.
        | "ssh" | "sftp" | "kubectl" | "docker" | "podman" | "helm" | "systemctl"
        | "launchctl" | "adb" | "xcrun" | "curl" | "wget" | "gh" | "aws" | "rclone"
        | "restic" | "borg"
        // Build and package tooling: it reads whole trees by convention rather
        // than by argument, so its operands are targets, never paths.
        // Attributing a repository to whoever ran `cargo test` in it would make
        // every session an owner of everything it built.
        | "cargo" | "rustc" | "npm" | "pnpm" | "yarn" | "npx" | "make" | "cmake" | "gradle"
        | "gradlew" | "mvn" | "pip" | "pip3" | "uv" | "poetry" | "go" | "ng" | "tsc" | "nix"
        | "nix-shell" | "nix-build" | "nix-env" | "nixos-rebuild" | "direnv" | "brew"
        | "flutter" | "dart" | "swift" | "javac" | "kotlinc"
        // Loop and conditional keywords, which the grammar leaves as ordinary
        // words on purpose (`echo done` must not end a loop).
        | "for" | "done" | "fi" | "esac" | "case" | "in" | "break" | "continue" | "return"
        | "select" | "function" | "[" | "[[" | "test" => Verb::NoFiles,

        _ => return None,
    })
}

/// What this command does, resolved against the directory it ran in.
///
/// One command in, one operation out. Wrappers and assignments are stripped
/// first, so `sudo nohup rm -rf x` is the `rm` it is.
pub fn classify(argv: &[String], cwd: Option<&str>, home: &str) -> Op {
    let argv = unwrap_command(argv);
    let Some(head) = argv.first() else {
        return Op::Nothing;
    };
    let name = basename(head);
    // A command invoked by path is itself a file that was used — but only when
    // nothing better is known about it. Counting the *binary* of a known command
    // put `.venv/bin/python` among the busiest paths in the corpus at 335 reads,
    // which says nothing about anybody's work; the script it runs does.
    let invoked_by_path = head
        .contains('/')
        .then(|| resolve(head, cwd, home))
        .flatten();

    let Some(verb) = verb(name) else {
        return match invoked_by_path {
            Some(script) => Op::Run { script },
            None => Op::Unknown {
                name: name.to_string(),
            },
        };
    };
    match (act(verb, argv, cwd, home), invoked_by_path) {
        (Op::Nothing, Some(script)) => Op::Run { script },
        (op, _) => op,
    }
}

/// The operation a verb performs on these arguments.
fn act(verb: Verb, argv: &[String], cwd: Option<&str>, home: &str) -> Op {
    // A program given by a flag leaves every operand a file, and the file it was
    // given from is itself read.
    let flags = match verb {
        Verb::Search(flags) | Verb::Stream { flags, .. } => flags,
        Verb::Interpreter(flags) | Verb::Walk(flags) => flags,
        Verb::Copy(flags) | Verb::Move(flags) => flags,
        _ => Flags::NONE,
    };
    let from_flag = has_flag(argv, flags.script);
    let script_files = paths(&flag_values(argv, flags.script_file), cwd, home);
    let words = operands(argv, flags.valued);
    // With the program supplied by a flag there is no leading operand to skip.
    let (leading, rest) = match (from_flag, words.split_first()) {
        (false, Some((first, rest))) => ((*first).to_string(), rest),
        _ => (String::new(), &words[..]),
    };

    match verb {
        Verb::Read => Op::Read {
            paths: paths(&words, cwd, home),
        },
        Verb::Search(_) => {
            // The file a pattern came from is an argument like any other, so it
            // keeps its place in front of the operands.
            let mut found = script_files;
            found.extend(paths(rest, cwd, home));
            Op::Search {
                pattern: leading,
                paths: found,
            }
        }
        Verb::Stream { honours_i, .. } => Op::Transform {
            program: leading,
            program_file: script_files.into_iter().next(),
            paths: paths(rest, cwd, home),
            in_place: honours_i
                && argv
                    .iter()
                    .any(|a| a.starts_with("-i") && !a.starts_with("--")),
        },
        Verb::Remove => Op::Remove {
            paths: paths(&words, cwd, home),
            recursive: argv
                .iter()
                .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('r'))
                || has_flag(argv, &["--recursive"]),
        },
        Verb::Overwrite => Op::Write {
            paths: paths(&words, cwd, home),
        },
        Verb::Copy(_) | Verb::Move(_) => {
            let Some((last, sources)) = words.split_last() else {
                return Op::Nothing;
            };
            let from = paths(sources, cwd, home);
            // A destination that is not a usable path — `host:dir` on a remote
            // copy — leaves the sources, which were still read here.
            match looks_like_path(last)
                .then(|| resolve(last, cwd, home))
                .flatten()
            {
                Some(to) if matches!(verb, Verb::Move(_)) => Op::Move { from, to },
                Some(to) => Op::Copy { from, to },
                None => Op::Read { paths: from },
            }
        }
        Verb::Interpreter(_) | Verb::Walk(_) => {
            let first = words.first().filter(|w| looks_like_path(w));
            match first.and_then(|w| resolve(w, cwd, home)) {
                // `find .` looked *in* its operand; an interpreter *ran* its own.
                Some(path) if matches!(verb, Verb::Walk(_)) => Op::Read { paths: vec![path] },
                Some(script) => Op::Run { script },
                None => Op::Nothing,
            }
        }
        Verb::ChangeDir => match words.first() {
            // An unresolvable target must make the directory *unknown*, never
            // leave it stale — carrying on with the old one resolves every later
            // relative path somewhere the command never ran.
            Some(word) => match resolve(word, cwd, home) {
                Some(to) => Op::ChangeDir { to: Some(to) },
                None => Op::Unknown {
                    name: "cd".to_string(),
                },
            },
            None => Op::ChangeDir {
                to: Some(home.to_string()),
            },
        },
        Verb::Git => git(argv, cwd, home),
        Verb::NoFiles => Op::Nothing,
    }
}

/// A `git` invocation, which needs its own reading for two reasons the general
/// shape cannot express: `-C <dir>` moves the directory its operands resolve
/// against, and most subcommands take *revisions* where a path would go —
/// `git diff origin/main` would otherwise record a file of that name.
fn git(argv: &[String], cwd: Option<&str>, home: &str) -> Op {
    let mut base = cwd.map(str::to_string);
    let mut rest = argv.iter().skip(1);
    let mut sub = None;
    while let Some(word) = rest.next() {
        match word.as_str() {
            "-C" => {
                if let Some(dir) = rest.next() {
                    base = resolve(dir, base.as_deref(), home);
                }
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                rest.next();
            }
            flag if flag.starts_with('-') => {}
            other => {
                sub = Some(other.to_string());
                break;
            }
        }
    }
    let Some(sub) = sub else {
        return Op::Nothing;
    };
    let words: Vec<&str> = rest
        .map(String::as_str)
        .skip_while(|w| w.starts_with('-') && *w != "--")
        .collect();
    let after_sep = words.iter().position(|w| *w == "--");
    let base = base.as_deref();
    match (sub.as_str(), after_sep) {
        ("add" | "stage", _) => Op::Git(GitOp::Stage {
            paths: paths(&words, base, home),
        }),
        // `rm` deletes, `mv` renames, `restore` overwrites — all real changes.
        // `checkout` is deliberately absent: it takes a branch as often as a
        // path, and `git checkout origin/main` would file a write against a file
        // of that name. It is readable only in its `--` form.
        ("rm" | "restore" | "mv", _) => Op::Git(GitOp::Alter {
            paths: paths(&words, base, home),
        }),
        // The separator is the author saying these are paths, which is exactly
        // the guarantee needed.
        (_, Some(at)) => Op::Git(GitOp::Inspect {
            paths: paths(&words[at + 1..], base, home),
        }),
        _ => Op::Git(GitOp::Other { subcommand: sub }),
    }
}

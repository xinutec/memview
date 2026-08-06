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
//!
//! ⚠ **Two of those are principles and two are limitations, and the difference
//! decides which may be lifted.**
//!
//! *Principles, permanent:* nothing is looked up on disk — the filesystem of the
//! day is gone and today's is a different machine's answer — and **nothing is
//! ever guessed**. A subject we cannot determine is recorded as undetermined,
//! never approximated, because an invented path is the one failure that makes
//! every count downstream a lie.
//!
//! *Limitations, to be lifted:* **nothing is bound and nothing is evaluated.**
//! There is no environment here, so `ADB=/nix/store/…/adb` two words earlier
//! cannot resolve the `$ADB` that follows — measured over the corpus, **564 of
//! the 1,023 commands using `$ADB` assign it in the same command text**, and it
//! is the largest unread name there is. Likewise a loop over a literal word list
//! (3,078 of 10,398) or over `$(seq N M)` with constant bounds (1,008) is fully
//! determined by the text and is read as nothing at all. That is a gap in the
//! model, not a limit on what is knowable, and closing it is the work in the
//! README's Roadmap.

use std::collections::BTreeMap;

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
    /// Runs shell script text **in this same shell** — `bash -c '…'`,
    /// `nix-shell --run '…'`. The text is parsed and classified in turn by
    /// [`crate::shell_files::extract`], which is where the working directory
    /// lives; a `cd` inside stays inside, exactly as it does in a subshell.
    ///
    /// **`ssh host '…'` is [`Op::Remote`], not this** — the difference is which
    /// machine's filesystem the paths belong to.
    Nested { script: String },
    /// Runs Python: `python3 -c '…'`, or a program fed in on stdin by a
    /// heredoc — `python3 - <<'PY' … PY`.
    ///
    /// **The one non-shell language this reads**, and it is here because
    /// refusing to was costing the most: 7,494 calls, and 3,547 file writes
    /// inside them that nothing else in the fleet could see. Read by
    /// [`crate::python`], which is as restrictive about Python as this module is
    /// about the shell. `node -e` is still refused — nobody has measured a
    /// reason to read it.
    Python { source: String },
    /// A command run on another machine: `ssh host '…'`, `kubectl exec … -- …`,
    /// `docker exec …`.
    ///
    /// **Read, but never attributed here.** The script is parsed like any other
    /// — 8,666 of the corpus's `ssh` payloads are a quoted script, and what they
    /// do to `/etc/nixos/configuration.nix` is real knowledge — but every path
    /// in it belongs to `host`, and putting one in a local index would claim a
    /// file that does not exist on this machine. Refusing to *parse* it was the
    /// cruder version of that rule: it kept the index clean by knowing nothing.
    ///
    /// The remote working directory is unknown, so only absolute paths survive
    /// unless the script `cd`s somewhere first — which many do.
    Remote { host: String, script: String },
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

/// How a command names the machine it reaches, and where its payload starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    /// `ssh [flags] [user@]host cmd…` — the words after the host are joined
    /// with spaces and handed to the remote shell, which is exactly what ssh
    /// itself does with them.
    Ssh,
    /// `kubectl [flags] exec <resource> [flags] -- cmd…`. Any other subcommand
    /// reaches no shell and names no file.
    Kubectl,
    /// `docker exec [flags] <container> cmd…`.
    Docker,
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

/// A `NAME=value` word, split — the shell's assignment, whether it stands alone
/// or prefixes a command.
///
/// The name has to be a name: a leading `-` makes `--flag=value` a flag, and a
/// `/` makes `a/b=c` a path that happens to contain one. Both appear in the
/// corpus and neither binds anything.
pub fn assignment(word: &str) -> Option<(&str, &str)> {
    let (name, value) = word.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() || first == '_')
        .then(|| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .filter(|ok| *ok)
        .map(|_| (name, value))
}

/// Put the values this scope has bound back into a word.
///
/// ⚠ **A name nobody bound is left exactly as it was**, which matters more than
/// it looks: `resolve` refuses a word still holding a `$`, so an unexpanded
/// variable stays refused rather than becoming a path named `$ADB`. Expansion
/// can only ever turn a refusal into a resolution, never the reverse.
///
/// `${NAME}` as well as `$NAME`, because both are written here. Nothing else of
/// the shell's expansion vocabulary — no `${NAME:-default}`, no `$*` — since a
/// half-understood expansion is the way to invent a path.
pub fn expand(word: &str, env: &BTreeMap<String, String>) -> String {
    if !word.contains('$') || env.is_empty() {
        return word.to_string();
    }
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (name, tail) = if let Some(braced) = after.strip_prefix('{') {
            match braced.split_once('}') {
                Some((name, tail)) => (name, tail),
                None => break,
            }
        } else {
            let end = after
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        match env.get(name) {
            Some(value) => out.push_str(value),
            // Put it back the way it was written, braces and all.
            None if after.starts_with('{') => {
                out.push_str("${");
                out.push_str(name);
                out.push('}');
            }
            None => {
                out.push('$');
                out.push_str(name);
            }
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Turn a word into an absolute path, or refuse it.
///
/// Refuses more than it accepts, and each refusal is a category that would
/// otherwise put a wrong path in the index:
/// - an unexpanded `$VAR` — because there is nowhere to keep a binding, not
///   because the value is unknowable; see the module doc. Refusing stays right
///   until there is: a variable read as its own name files work against a path
///   called `$ADB`;
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

/// The words after the first of `flags` to appear — the command a devshell
/// wrapper was asked to run.
fn after_flag<'a>(argv: &'a [String], flags: &[&str]) -> Option<&'a [String]> {
    argv.iter()
        .position(|word| flags.contains(&word.as_str()))
        .map(|at| &argv[at + 1..])
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
        // These run the real tool, which is the one worth classifying:
        // `npx biome check --write x.ts` is a `biome`, and 3,785 calls went
        // unread while `npx` stood in front of it.
        "npx" | "pnpx" | "bunx" => &[],
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
    /// Runs its first operand, which is a script — unless one of `inline` is
    /// given, in which case its value is shell text to be read in turn.
    ///
    /// `inline` is empty for `node`: its `-e` carries JavaScript, and reading
    /// that as shell would invent commands nobody ran.
    Interpreter {
        flags: Flags,
        inline: &'static [&'static str],
    },
    /// Runs Python — from `-c`, from a heredoc, or from a script file.
    ///
    /// Its own verb rather than an [`Verb::Interpreter`] with a flag, because
    /// what it does with a file is decided by reading the *program*, and no
    /// other interpreter here is read at all.
    Python,
    /// Runs shell text given as the value of a flag: `nix-shell --run '…'`.
    Script(&'static [&'static str]),
    /// The words *after* one of these flags are themselves a command:
    /// `nix develop -c npm run verify`. Not a string to parse — an argv to
    /// classify, so it costs no second parse and cannot fail one.
    Carries(&'static [&'static str]),
    /// Looked in its first operand; everything after is an expression.
    Walk(Flags),
    /// A checker over path operands — a linter, a formatter, a type checker.
    /// Reads them, unless one of `writes` is given, in which case it rewrites
    /// them in place: `biome check --write x.ts`, `ktlint -F`.
    ///
    /// These were invisible until the devshell wrappers were read, and then
    /// they were the whole top of the unknown list: 1,458 `ruff`, 1,058
    /// `ktlint`, 822 `mypy`.
    Check {
        flags: Flags,
        writes: &'static [&'static str],
    },
    /// Moves the working directory.
    ChangeDir,
    /// Needs its own reading — revisions sit where paths would.
    Git,
    /// Hands a script to another machine.
    Remote(Remote),
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

        "python" | "python3" => Verb::Python,
        // An interpreter's flags carry code or a module name, never a path.
        "node" | "deno" | "bun" => Verb::Interpreter {
            flags: Flags::valued(&["-e", "-p", "--eval"]),
            inline: &[],
        },
        // The one family whose `-c` really is shell.
        "bash" | "sh" | "zsh" | "dash" | "ksh" => Verb::Interpreter {
            flags: Flags::valued(&["-c", "-o"]),
            inline: &["-c"],
        },
        "ruby" | "perl" => Verb::Interpreter {
            flags: Flags::valued(&["-e", "-E", "-I"]),
            inline: &[],
        },
        "source" | "." | "sqlite3" => Verb::Interpreter {
            flags: Flags::NONE,
            inline: &[],
        },

        "find" | "fd" => Verb::Walk(Flags::valued(&[
            "-name", "-iname", "-path", "-type", "-exec",
        ])),
        // Checkers and formatters. `--fix`/`--write`/`-F` is the difference
        // between reading a file and rewriting it, exactly as `-i` is for sed.
        "ruff" => Verb::Check {
            flags: Flags::valued(&["--config", "--select", "--ignore"]),
            writes: &["--fix", "--fix-only"],
        },
        "biome" | "prettier" | "eslint" | "stylelint" => Verb::Check {
            flags: Flags::valued(&["--config", "--config-path", "--ext"]),
            writes: &["--write", "--fix"],
        },
        "ktlint" => Verb::Check {
            flags: Flags::NONE,
            writes: &["-F", "--format"],
        },
        "black" | "isort" => Verb::Check {
            flags: Flags::valued(&["--line-length"]),
            // These rewrite by default; `--check`/`--diff` is what makes them
            // read-only, so the absence of a flag means a write. Stated as the
            // exception it is rather than folded in with the others.
            writes: &[],
        },
        "mypy" | "pytest" | "shellcheck" | "pyright" | "clang-format" | "tsc" => Verb::Check {
            flags: Flags::valued(&["--config-file", "-p", "--project", "-k", "--python-version"]),
            writes: &[],
        },
        // The JavaScript test runners, and the top of the unread list after the
        // one name nothing can ever resolve: `vitest` 1,412 and `playwright`
        // 1,330 calls, measured 2026-08-06. Their operands are spec files and
        // nothing more — no grammar was needed for either, which is why they went
        // unread for so long behind the assumption that JavaScript meant a
        // parser. `node -e` really does need one and is worth 23 writes.
        //
        // Both rewrite on demand: a snapshot update is a real change to a real
        // file, and it is the only way either of them writes anything.
        "vitest" => Verb::Check {
            flags: Flags::valued(&["--config", "-c", "--reporter", "-t", "--testNamePattern"]),
            writes: &["-u", "--update"],
        },
        "playwright" => Verb::Check {
            flags: Flags::valued(&["--config", "-c", "--project", "--reporter", "--grep", "-g"]),
            writes: &["-u", "--update-snapshots"],
        },
        // Runs a TypeScript file the way `node` runs a JavaScript one. An
        // interpreter rather than a checker: its operand is a program, not a
        // subject, and what that program does to files is beyond this reader —
        // the same refusal `node` gets, for the same reason.
        "tsx" => Verb::Interpreter {
            flags: Flags::valued(&["-e", "--eval", "--tsconfig"]),
            inline: &[],
        },
        "cd" => Verb::ChangeDir,
        "git" => Verb::Git,

        // The devshell wrappers. Between them they carry a third of the
        // corpus's commands, and every one was invisible while the shell they
        // open went unread: 15,366 `nix … -c`, 8,870 `nix-shell --run`.
        "nix" => Verb::Carries(&["-c", "--command"]),
        "nix-shell" => Verb::Script(&["--run"]),

        "ssh" => Verb::Remote(Remote::Ssh),
        "kubectl" => Verb::Remote(Remote::Kubectl),
        "docker" => Verb::Remote(Remote::Docker),

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
        | "sftp" | "podman" | "helm" | "systemctl"
        | "launchctl" | "adb" | "xcrun" | "curl" | "wget" | "gh" | "aws" | "rclone"
        | "restic" | "borg"
        // Build and package tooling: it reads whole trees by convention rather
        // than by argument, so its operands are targets, never paths.
        // Attributing a repository to whoever ran `cargo test` in it would make
        // every session an owner of everything it built.
        | "cargo" | "rustc" | "npm" | "pnpm" | "yarn" | "make" | "cmake" | "gradle"
        | "gradlew" | "mvn" | "pip" | "pip3" | "uv" | "poetry" | "go" | "ng"
        | "nix-build" | "nix-env" | "nixos-rebuild" | "direnv" | "brew"
        | "flutter" | "dart" | "swift" | "javac" | "kotlinc"
        // Build tools that take targets rather than paths, like cargo.
        | "lake" | "mariadb" | "mysql" | "psql"
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
///
/// `heredocs` are the bodies the command was given on stdin. For nearly every
/// command they are data and go unread; for `python3 -` the body *is* the
/// program, and that is the only reason they are carried this far.
pub fn classify(argv: &[String], heredocs: &[String], cwd: Option<&str>, home: &str) -> Op {
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
    match (act(verb, argv, heredocs, cwd, home), invoked_by_path) {
        (Op::Nothing, Some(script)) => Op::Run { script },
        (op, _) => op,
    }
}

/// The operation a verb performs on these arguments.
fn act(verb: Verb, argv: &[String], heredocs: &[String], cwd: Option<&str>, home: &str) -> Op {
    // A program given by a flag leaves every operand a file, and the file it was
    // given from is itself read.
    let flags = match verb {
        Verb::Search(flags) | Verb::Stream { flags, .. } | Verb::Check { flags, .. } => flags,
        Verb::Interpreter { flags, .. } | Verb::Walk(flags) => flags,
        Verb::Copy(flags) | Verb::Move(flags) => flags,
        // `-c` is the program, `-m` a module, `-W` a warning filter: none of
        // them is an operand, and the first is the whole point.
        Verb::Python => Flags::valued(&["-c", "-m", "-W"]),
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
        Verb::Script(flags) => match flag_values(argv, flags).first() {
            Some(script) => Op::Nested {
                script: (*script).to_string(),
            },
            None => Op::Nothing,
        },
        Verb::Carries(flags) => match after_flag(argv, flags) {
            // The rest of the line is a command in its own right. Classified
            // rather than re-parsed: it is already words. The heredoc travels
            // with it — `nix develop -c python3 - <<'PY'` opens the body for
            // the python, not for the wrapper.
            Some(rest) if !rest.is_empty() => classify(rest, heredocs, cwd, home),
            _ => Op::Nothing,
        },
        Verb::Python => {
            // A script file is the program, and a heredoc alongside it is that
            // program's *input* — reading it as source would attribute the
            // data's own paths to a program that never named them.
            if let Some(code) = flag_values(argv, &["-c"]).first() {
                return Op::Python {
                    source: (*code).to_string(),
                };
            }
            match words
                .first()
                .filter(|word| looks_like_path(word))
                .and_then(|word| resolve(word, cwd, home))
            {
                Some(script) => Op::Run { script },
                // `python3 - <<'PY'`, and `python3 <<'PY'` — with no file to
                // run, the body on stdin is the program.
                None => match heredocs.first() {
                    Some(source) => Op::Python {
                        source: source.clone(),
                    },
                    None => Op::Nothing,
                },
            }
        }
        Verb::Interpreter { inline, .. } if !flag_values(argv, inline).is_empty() => Op::Nested {
            script: flag_values(argv, inline)
                .first()
                .map(|s| (*s).to_string())
                .unwrap_or_default(),
        },
        Verb::Interpreter { .. } | Verb::Walk(_) => {
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
            Some(word) if repeats(word, cwd) => Op::ChangeDir {
                to: cwd.map(str::to_string),
            },
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
        Verb::Check { writes, .. } => {
            let paths = paths(&words, cwd, home);
            // `black`/`isort` rewrite unless told to check, which is why an
            // empty `writes` means "writes by default" for them alone — the
            // list says which flag turns writing ON, and they have none.
            let rewrites = if writes.is_empty() {
                false
            } else {
                has_flag(argv, writes)
            };
            if rewrites {
                Op::Write { paths }
            } else {
                Op::Read { paths }
            }
        }
        Verb::Git => git(argv, cwd, home),
        Verb::Remote(kind) => remote(kind, argv),
        Verb::NoFiles => Op::Nothing,
    }
}

/// Whether a `cd` would step into the directory it is already standing in —
/// `cd android` from `…/observe/android`.
///
/// **Measured, not guessed.** The corpus holds 90 such calls, and their tool
/// results say the doubling never happened, by two different mechanisms: in 33
/// the `cd` itself failed (`cd: no such file or directory: android`) because
/// the session had already moved there earlier and the agent said so again; in
/// the other 57 the command succeeded, which means the transcript's recorded
/// `cwd` was already the directory this `cd` was about to enter. Applying the
/// move is wrong in both — and taking it as a no-op is right in both, which is
/// why one rule covers 90 of 90.
///
/// Relative targets only. `cd /Users/pippijn/Code/observe/android` from inside
/// it is an ordinary, and truthful, no-op that needs no special reading.
fn repeats(word: &str, cwd: Option<&str>) -> bool {
    let target = word.trim_end_matches('/');
    !target.is_empty()
        && !target.starts_with(['/', '~', '$', '.'])
        && cwd.is_some_and(|cwd| cwd.trim_end_matches('/').ends_with(&format!("/{target}")))
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

/// Flags of `ssh` that consume the following word, so a value is never mistaken
/// for the host.
const SSH_VALUED: &[&str] = &[
    "-o", "-p", "-i", "-l", "-F", "-L", "-R", "-D", "-J", "-E", "-b", "-c", "-m", "-O", "-Q", "-S",
    "-W", "-w",
];

/// The machine a command reaches, and the script it hands over.
///
/// Returns [`Op::Nothing`] when there is no script — `ssh host` alone opens a
/// session nobody scripted, and `kubectl get pods` reaches no shell at all.
fn remote(kind: Remote, argv: &[String]) -> Op {
    let (host, script) = match kind {
        Remote::Ssh => {
            let mut rest = argv.iter().skip(1);
            let mut host = None;
            while let Some(word) = rest.next() {
                if SSH_VALUED.contains(&word.as_str()) {
                    rest.next();
                } else if !word.starts_with('-') {
                    host = Some(word.clone());
                    break;
                }
            }
            // ssh joins its remaining arguments with spaces and gives them to
            // the remote shell, so joining them here is not an approximation.
            let script = rest.cloned().collect::<Vec<_>>().join(" ");
            (host, script)
        }
        Remote::Kubectl | Remote::Docker => {
            let mut words = argv.iter().skip(1).peekable();
            let mut target = None;
            let mut saw_exec = false;
            let mut script = Vec::new();
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--" if saw_exec => {
                        script = words.map(String::clone).collect();
                        break;
                    }
                    "exec" => saw_exec = true,
                    // `-n ns`, `-c container`, `--namespace=x`.
                    "-n" | "-c" | "--namespace" | "--container" | "-u" | "-e" | "-w" => {
                        words.next();
                    }
                    flag if flag.starts_with('-') => {}
                    other if saw_exec && target.is_none() => target = Some(other.to_string()),
                    // `docker exec name sh -c '…'` has no `--` separator.
                    _ if saw_exec && kind == Remote::Docker => {
                        script = std::iter::once(word.clone())
                            .chain(words.map(String::clone))
                            .collect();
                        break;
                    }
                    _ => {}
                }
            }
            // **Not joined like ssh's.** `kubectl exec` and `docker exec` hand
            // their payload to `exec()` as an argv, with no shell to re-split
            // it, so `sh -c 'cat a b'` is three words and the third keeps its
            // spaces. Joining would hand `cat` alone to the inner shell and
            // lose the rest. The `sh -c` shape is what the corpus writes.
            let script = match script.split_first() {
                Some((program, rest))
                    if matches!(basename(program), "sh" | "bash" | "zsh" | "dash") =>
                {
                    flag_values(
                        &std::iter::once(program.clone())
                            .chain(rest.iter().cloned())
                            .collect::<Vec<_>>(),
                        &["-c"],
                    )
                    .first()
                    .map(|s| (*s).to_string())
                    .unwrap_or_else(|| rest.join(" "))
                }
                _ => script.join(" "),
            };
            (target, script)
        }
    };
    match (host, script) {
        // A host named by a variable — `ssh "$h" '…'` — cannot be resolved to a
        // machine, and a use filed against `$h` is filed against nothing. The
        // command is dropped rather than attributed to a name that is not one.
        (Some(host), _) if host.contains('$') => Op::Nothing,
        (Some(host), script) if !script.trim().is_empty() => Op::Remote {
            host: machine(&host),
            script,
        },
        _ => Op::Nothing,
    }
}

/// The machine a target names: `root@isis.xinutec.org`, `root@isis` and `isis`
/// are one host, and the corpus writes all three.
fn machine(target: &str) -> String {
    let after_user = target.rsplit('@').next().unwrap_or(target);
    // An address is not a name with a domain on it: taking the first label of
    // `192.168.1.133` gives a host called `192`, and three machines share it.
    if after_user
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    {
        return after_user.to_string();
    }
    after_user
        .split('.')
        .next()
        .unwrap_or(after_user)
        .to_string()
}

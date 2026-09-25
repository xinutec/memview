//! What a shell command *does*, as a type rather than a table lookup.
//!
//! [`crate::shell`] gives words; this gives meaning. The first version went straight
//! from words to "which paths, read or written", and that projection threw away
//! everything else the command said: `grep hsmmDecode src/x.ts` became one read,
//! indistinguishable from `cat src/x.ts`, though only one of them records *what was
//! being looked for*.
//!
//! Naming the operation keeps that. A [`Op::Search`] knows its pattern, a
//! [`Op::Move`] knows a file's old name as well as its new one. The file use
//! [`crate::shell_files`] mines is then a *projection* of these rather than a second
//! table to keep in step with this one.
//!
//! Everything the older table refused, this refuses identically: an unknown command
//! is [`Op::Unknown`] and contributes nothing, nothing is expanded beyond
//! `~`/`$HOME`, nothing is looked up on disk, and a word must be shaped like a path
//! before it can be one.
//!
//! Two of those are principles and one was a limitation. *Permanent:* nothing
//! is looked up on disk — the filesystem of the day is gone — and **nothing is ever
//! guessed**, because an invented path makes every count downstream a lie. *Now a
//! measurement:* a subject that cannot be determined used to vanish instead of
//! counting, so a command that named an unreadable file looked exactly like one that
//! named none. Those subjects are collected — see [`paths`] and [`undetermined`].
//!
//! Counting them is the whole of what can be done about them: a glob is answered by
//! the filesystem of the day and a `$(…)` by running something, and neither is
//! available here or ever will be.

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
    /// [`crate::shell_files::extract`], where the working directory lives; a `cd` inside
    /// stays inside.
    ///
    /// **`ssh host '…'` is [`Op::Remote`], not this** — the difference is which
    /// machine's filesystem the paths belong to.
    Nested { script: String },
    /// Runs Python: `python3 -c '…'`, or a program fed in on stdin by a heredoc.
    ///
    /// **The first non-shell language this read**, and it is here because refusing to
    /// was costing the most: thousands of file writes inside Python that nothing else in
    /// the fleet could see. Read by [`crate::python`], which is as restrictive about
    /// Python as this module is about the shell.
    Python { source: String },
    /// Runs JavaScript: `node -e '…'`, `node --input-type=module -e '…'`, or a program
    /// fed in on stdin by a heredoc.
    ///
    /// An earlier ranking dismissed it — "`node -e` is a query tool, not an editor" — by
    /// counting `node -e` alone, by distinct payload, and counting no reads. Counted
    /// across every runtime and every program passed in a flag, the reads and writes are
    /// there. Read by [`crate::javascript`].
    JavaScript { source: String },
    /// Statements sent to a database: `mariadb -e '…'`, `sqlite3 x.db '…'`, or a
    /// heredoc on stdin. Read by [`crate::sql`].
    ///
    /// `database` is a FILE only for sqlite3. `mariadb health` names a database on
    /// a server — a name in a catalogue, not a path — and resolving it against the
    /// working directory would invent a file that has never existed.
    ///
    /// Whether that file is read or written is decided by the STATEMENTS, not by the
    /// command: the argv of a `SELECT` and a `DELETE` are identical in shape.
    Sql {
        source: String,
        database: Vec<String>,
    },
    /// A command run on another machine: `ssh host '…'`, `kubectl exec … -- …`,
    /// `docker exec …`.
    ///
    /// **Read, but never attributed here.** The script is parsed like any other — what
    /// an `ssh` payload does to `/etc/nixos/configuration.nix` is real knowledge — but
    /// every path in it belongs to `host`, and putting one in a local index would claim
    /// a file that does not exist on this machine. Refusing to *parse* it was the cruder
    /// version of that rule: it kept the index clean by knowing nothing.
    ///
    /// The remote working directory is unknown, so only absolute paths survive unless
    /// the script `cd`s somewhere first.
    Remote { host: String, script: String },
    /// A program run on another machine **with no shell anywhere**:
    /// `kubectl exec pod -- mariadb -e 'SELECT …'`, `docker exec c ls /etc`.
    ///
    /// The distinction [`Op::Remote`] cannot express, and getting it wrong accounted
    /// for nearly every nested refusal (memview#1028). `kubectl exec` and `docker
    /// exec` hand their words to `exec()`; nothing re-splits them and nothing removes a
    /// quote. Joining them back into one string and parsing that as shell put SQL and
    /// JavaScript in front of the shell grammar.
    ///
    /// So the payload stays an **argv to classify**, never text to parse — the same
    /// choice [`Verb::Carries`] makes, and for the same reason. `-- sh -c '…'` is still
    /// [`Op::Remote`]: there the shell is real and its argument really is a script.
    RemoteRun { host: String, argv: Vec<String> },
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
    /// `git add` — **staging, which changes no file.** Named rather than dropped: it
    /// was a large fraction of every shell-derived write when it was miscounted as one,
    /// and a variant that exists but projects to nothing keeps that decision visible.
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
/// stray context number, a git refspec, the word after a flag whose value-taking
/// this table does not know about. Requiring a slash, a `~`, or an extension throws
/// those away.
///
/// It costs something real and known: `rg foo src` loses `src`, because a bare
/// directory name is indistinguishable from a bare non-path. That is the side of the
/// trade to be on — a lost read is an undercount, a kept non-path is a fabrication.
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
/// A name nobody bound is left exactly as it was. `resolve` refuses a word
/// still holding a `$`, so an unexpanded variable stays refused rather than becoming
/// a path named `$ADB`. Expansion can only ever turn a refusal into a resolution,
/// never the reverse.
///
/// `${NAME}` as well as `$NAME`, because both are written here. Nothing else of the
/// shell's expansion vocabulary — a half-understood expansion is the way to invent a
/// path.
pub fn expand(word: &str, env: &BTreeMap<String, String>) -> String {
    expand_marking(word, env).0
}

/// [`expand`], and where in the answer a value was actually substituted.
///
/// Only these spans may word-split, and that is bash's rule rather than a
/// refinement of it. Splitting happens to the characters an expansion *produced*,
/// never to the literal text beside them in the same word:
///
/// ```text
/// awk '/^# page 1/{p=1;next} {print > "'$S'/'$v'.pg"}' $S/$v.words
/// ```
///
/// is ONE word — `$S` and `$v` hold no whitespace — and splitting the whole of it
/// shredded a single-quoted `awk` program into fragments and filed them as files
/// (#1195). The spans are byte ranges into the returned string, in order and
/// non-overlapping.
///
/// A substitution nobody could evaluate produces no span, so nothing inside
/// `$(cat list.txt)` splits either: its text is not its output.
pub fn expand_marking(word: &str, env: &BTreeMap<String, String>) -> (String, Vec<(usize, usize)>) {
    if !word.contains('$') || env.is_empty() {
        return (word.to_string(), Vec::new());
    }
    let mut produced: Vec<(usize, usize)> = Vec::new();
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
            Some(value) => {
                let from = out.len();
                out.push_str(value);
                produced.push((from, out.len()));
            }
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
    (out, produced)
}

/// Turn a word into an absolute path, or refuse it.
///
/// Refuses more than it accepts, and each refusal is a category that would otherwise
/// put a wrong path in the index:
/// - an unexpanded `$VAR` — every value the binder knows is already in the word by
///   now, so what is left is a name nobody bound, one bound twice, or one only
///   running the command would answer;
/// - `host:path` and anything with a scheme — another machine, or a URL;
/// - `/dev/*`, which is plumbing: left in, `/dev/null` is the busiest path in the
///   whole corpus and says nothing about anyone's work;
/// - anything at all when the working directory is unknown, since a relative path
///   without one names nothing.
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

/// The paths among these words, resolved and in order — and, into `unnamed`, the
/// subjects that were refused because the text does not determine them.
///
/// A refusal used to leave no trace, which made an unknown look like an
/// absence. `wc -l "$f"` inside a glob loop recorded exactly what `wc -l` with no
/// operand records: nothing. The first is a file this reader cannot name, the second
/// is a command that named none. See [`undetermined`].
fn paths(unnamed: &mut Vec<String>, words: &[&str], cwd: Option<&str>, home: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in words {
        match looks_like_path(word)
            .then(|| resolve(word, cwd, home))
            .flatten()
        {
            Some(path) => out.push(path),
            None if undetermined(word) => unnamed.push((*word).to_string()),
            None => {}
        }
    }
    out
}

/// Whether a refused word is a subject the text does not determine, as opposed to
/// one that is simply not a file.
///
/// The distinction is the whole value of the count. Most refusals are correct
/// and uninteresting: `rg pattern src` loses `src` because a bare word cannot be
/// told from a bare directory, `-` is stdin, a git refspec is not a path. Counting
/// those would bury the ones that matter under noise nobody can act on.
///
/// A surviving `$` — or a backtick, since `` `which claude` `` carries no `$` — is
/// the marker, because by this point every expansion the text determines has already
/// been made. What is left is a value that was never in the text.
///
/// A relative path under an unknown directory is NOT counted here, though it is
/// equally unnameable. That refusal has a different cause and a different remedy, and
/// folding the two together would make a count that cannot be acted on either way.
///
/// A word that spans lines is a program body, not a path. A template literal
/// carries `${…}`, the very marker this reads as an unmade expansion, so source text
/// arrives wearing the costume of an unnameable subject. Dropped rather than
/// counted: the refusal is correct and there is nothing to act on.
fn undetermined(word: &str) -> bool {
    word.contains(['$', '`']) && !only_arithmetic(word) && !word.contains('\n')
}

/// Whether a word is arithmetic and nothing a path could be made of.
///
/// An arithmetic expansion evaluates to a NUMBER, so it was never a candidate
/// for being a file — and they sat in the count of subjects the reader could not
/// name, which is the figure both apps print as the honest limit of what it knows.
///
/// Narrow on purpose: `/tmp/$((n)).txt` IS a path and must stay a subject. The
/// test is not "contains arithmetic" but "is arithmetic, with nothing path-shaped
/// around it" — erring wide would delete real subjects to make a number look better.
fn only_arithmetic(word: &str) -> bool {
    if !word.contains("$((") || word.contains(['/', '~']) {
        return false;
    }
    // What is left once every `$(( … ))` is removed. A path would still have
    // something of its own here; an arithmetic operand has punctuation at most.
    let mut rest = String::new();
    let bytes = word.as_bytes();
    let mut i = 0;
    while i < word.len() {
        if word[i..].starts_with("$((") {
            // Walk to the parenthesis that closes the one after the `$`, so a
            // nested `$(( (a+b) * c ))` is skipped whole rather than truncated.
            let mut depth = 0usize;
            let mut j = i + 1;
            while j < word.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            i = j.saturating_add(1).min(word.len());
            continue;
        }
        let c = word[i..].chars().next().expect("in bounds");
        rest.push(c);
        i += c.len_utf8();
    }
    rest.chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '*' | ' ' | '.' | ',' | ':' | '='))
}

/// The operands of a command: its words with the program and its flags removed.
///
/// `--` ends the flags, `--flag=value` carries its own value, and a flag named in
/// `valued` eats the word after it. An empty word is dropped — kept, BSD
/// `sed -i '' 's/a/b/' f` offers `''` as the script to skip and the real script is
/// then recorded as a path, since it is full of slashes.
fn operands<'a>(argv: &'a [String], flags: &Flags) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = argv.iter().skip(1);
    let mut reading_flags = true;
    while let Some(word) = rest.next() {
        if reading_flags && word == "--" {
            reading_flags = false;
        } else if reading_flags && word.starts_with('-') && word.len() > 1 {
            // A pair flag eats two words; a valued one eats a single word.
            if flags.pair.contains(&word.as_str()) || flags.pair_file.contains(&word.as_str()) {
                rest.next();
                rest.next();
            } else if flags.valued.contains(&word.as_str()) {
                rest.next();
            }
        } else if !word.is_empty() {
            out.push(word.as_str());
        }
    }
    out
}

/// The files named by a `pair_file` flag — the second word of each pair.
fn paired_files<'a>(argv: &'a [String], pair_file: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = argv.iter().skip(1);
    while let Some(word) = rest.next() {
        if pair_file.contains(&word.as_str()) {
            // Step over the NAME; the word after it is the file.
            rest.next();
            if let Some(file) = rest.next() {
                out.push(file.as_str());
            }
        }
    }
    out
}

/// Whether one word spells `-i` as a FLAG, not as a character of some other flag's
/// attached value.
///
/// `perl -Itest/lib -e '…'` was in-place to a `contains('i')` test — the `i`
/// in `lib` — and `in_place` decides the operands' direction, so a file the command
/// read was recorded as one it rewrote. Surfaced by the concept census.
///
/// A cluster's letters end where a value-taking flag starts — `valued` is the verb's
/// own list, so `-I` ends perl's cluster and does not end sed's `-Ei`. Digits pass:
/// `-0pi` spells a record separator in front of in-place.
fn spells_in_place(word: &str, valued: &[&str]) -> bool {
    if !word.starts_with('-') || word.starts_with("--") {
        return false;
    }
    for c in word.chars().skip(1) {
        if c == 'i' {
            return true;
        }
        let takes_value = valued
            .iter()
            .any(|flag| flag.chars().count() == 2 && flag.ends_with(c));
        if takes_value || !(c.is_ascii_alphabetic() || c.is_ascii_digit()) {
            return false;
        }
    }
    false
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

/// The script a shell's `-c` carries, including when `c` closes a cluster of other
/// short flags.
///
/// `-lc` is not `-c`, and testing the token for equality misses it silently.
/// Bash reads `-l`, `-i` and `-c` as separate one-letter options, so `sh -lc 'x'` is
/// a login shell running `x`. Every corpus command spelling it that way was misread:
/// the `kubectl exec` arm fell back to joining the tail, which handed on
/// `-lc NUM=$(…)` AS the script. The reader then refused it with a reason about the
/// script's own text, so the fabricated word never appeared as a flag error — see
/// `feedback_no_masking_fallbacks`.
///
/// Only sound for the shell family, and only called there. A cluster is read
/// as letters, so `-exec` would qualify on shape alone; `find` never reaches this
/// because a verb decides first who is being asked.
pub fn shell_c_value(argv: &[String]) -> Option<&str> {
    let mut rest = argv.iter().skip(1);
    while let Some(word) = rest.next() {
        let Some(letters) = word.strip_prefix('-') else {
            continue;
        };
        if letters.ends_with('c') && letters.bytes().all(|b| b.is_ascii_lowercase()) {
            return rest.next().map(String::as_str);
        }
    }
    None
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
        // `npx biome check --write x.ts` is a `biome`, and thousands of calls went unread
        // while `npx` stood in front of it.
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
    /// Flags that consume the following two words, where the SECOND is a file the
    /// command reads.
    ///
    /// Skipping both would delete a real read. `jq --slurpfile a data.json
    /// '.filter'` loads `data.json`; today it survives only by accident, as a stray
    /// operand left over from the same shift `pair` fixes — so filing these under `pair`
    /// would tidy the operand list and lose genuine reads with it.
    pair_file: &'static [&'static str],
    /// Flags that consume the following **two** words — a name and a value.
    ///
    /// `jq --arg dt 2026-01-01 '.filter' data.json` was read with the operand list
    /// shifted by one. Skipping a single word left the VALUE as the first operand, so
    /// the date was recorded as the jq program and the real filter fell through to the
    /// path list — where its `$dt` made it an unnamed file subject. Two wrong facts from
    /// one missing word.
    pair: &'static [&'static str],
    /// Flags that supply the pattern or program.
    script: &'static [&'static str],
    /// Those among them naming a *file* of patterns, itself a read.
    script_file: &'static [&'static str],
}

impl Flags {
    const NONE: Flags = Flags {
        valued: &[],
        pair_file: &[],
        pair: &[],
        script: &[],
        script_file: &[],
    };
    const fn valued(valued: &'static [&'static str]) -> Flags {
        Flags {
            valued,
            pair_file: &[],
            pair: &[],
            script: &[],
            script_file: &[],
        }
    }
}

/// What kind of command this is — **the closed set, named once**.
///
/// The classifier below matches on this rather than on the command name, so the
/// string is parsed exactly once, at [`verb`], and a name nobody taught it cannot
/// slip through a `_ =>` arm pretending to be understood. Grouped by *behaviour*
/// rather than one variant per command.
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
    /// A database client. Its own verb rather than an [`Verb::Interpreter`]
    /// because its operand is not a program: for a server client it is a
    /// database NAME, and for sqlite3 it is the database file itself.
    Sql {
        /// Flags whose value is the statements to run.
        program: &'static [&'static str],
        /// Whether the first operand is a local database FILE.
        file_operand: bool,
    },
    /// Runs JavaScript — from `-e`, from a heredoc, or from a script file.
    ///
    /// Its own verb rather than an [`Verb::Interpreter`], for the same reason
    /// [`Verb::Python`] is: what it does with a file is decided by reading the
    /// *program*.
    JavaScript,
    /// Reads what its input flags name and writes its LAST operand.
    ///
    /// `ffmpeg -i in.wav out.wav`, and it is the only shape here where the output is
    /// positional and the inputs are not. The path guard is what makes "the last
    /// operand" safe: `-`, `pipe:1` and a bare `5` left over from an undeclared flag are
    /// not paths, so they name no write.
    Convert {
        input: &'static [&'static str],
        /// Flags naming the output. **Empty means the output is the LAST
        /// OPERAND**, which is how ffmpeg spells it and how nothing else here
        /// does — `dhall-to-json --file x --output y` names both ends in flags
        /// and has no operand at all.
        output: &'static [&'static str],
    },
    /// Reads the archive named first. Everything after it is a member pattern
    /// INSIDE the archive — `unzip x.zip 'FS/data/**' -d out` — which is not a
    /// file on this machine however much it looks like a path, and `-d` names a
    /// directory, which this table does not attribute.
    Archive,
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
    /// A checker over path operands — a linter, a formatter, a type checker. Reads
    /// them, unless one of `writes` is given, in which case it rewrites them in place:
    /// `biome check --write x.ts`, `ktlint -F`.
    ///
    /// These were invisible until the devshell wrappers were read, and then they were
    /// the whole top of the unknown list.
    Check {
        flags: Flags,
        writes: &'static [&'static str],
    },
    /// Fetches from the network, and names a local file only where a flag says to save
    /// into one: `curl -o x.json`, `wget -O x.json`.
    ///
    /// These were `NoFiles` once, and the asymmetry is what gave it away.
    /// `curl URL > file` was always counted, because a redirect is collected whatever
    /// the command is; `curl -o file URL` was not. Two spellings of one act, counted
    /// differently. The same shape as the `sed -e` defect: an operand given by a FLAG
    /// leaves nothing in the operand position to notice.
    ///
    /// Everything else about them stays invisible on purpose: a URL is not a path.
    Fetch {
        /// The flags whose *value* is the file written. `-O` appears in both
        /// tools and means opposite things — wget's takes the name, curl's takes
        /// none and derives it from the URL — so only the ones that carry a name
        /// are listed, and curl's `-O` is left to name nothing rather than
        /// resolve to a guess.
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
    /// Distinct from a name that is absent, and the distinction matters: without it the
    /// worklist of commands still to support is headed by `echo` forever, and tens of
    /// thousands of commands that were never going to name a file look like a gap in
    /// coverage.
    NoFiles,
}

/// Where a command's Python program is, for a reader that has to point at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonFrom {
    /// The word at this position in `argv`: `python3 -c '…'`.
    Argument(usize),
    /// Its standard input: `python3 - <<'PY'`, or `python3` with no operand.
    Stdin,
}

impl PythonFrom {
    fn shifted(self, by: usize) -> Self {
        match self {
            PythonFrom::Argument(at) => PythonFrom::Argument(at + by),
            PythonFrom::Stdin => PythonFrom::Stdin,
        }
    }
}

/// Where `argv` finds the Python program it runs, looking through the wrappers
/// [`classify`] looks through. `None` when it runs a script file, a module
/// (`-m`), or no Python at all.
///
/// A program handed to another shell (`bash -c`, `nix-shell --run`) or another
/// machine is a layer below this one: its text is parsed and asked in turn.
pub fn python_program(argv: &[String]) -> Option<PythonFrom> {
    let inner = unwrap_command(argv);
    let offset = argv.len() - inner.len();
    match verb(basename(inner.first()?))? {
        Verb::Python => {
            let mut at = 1;
            while let Some(word) = inner.get(at) {
                match word.as_str() {
                    "-c" => {
                        return inner
                            .get(at + 1)
                            .map(|_| PythonFrom::Argument(at + 1).shifted(offset));
                    }
                    "-m" => return None,
                    "-" | "/dev/stdin" => return Some(PythonFrom::Stdin),
                    "-W" | "-X" => at += 2,
                    flag if flag.starts_with('-') => at += 1,
                    _ => return None,
                }
            }
            Some(PythonFrom::Stdin)
        }
        Verb::Carries(flags) => {
            let at = inner
                .iter()
                .position(|word| flags.contains(&word.as_str()))?
                + 1;
            python_program(&inner[at..]).map(|from| from.shifted(at + offset))
        }
        _ => None,
    }
}

/// Whether a command name is a Python interpreter: `python`, `python3`,
/// `python3.12`.
///
/// A dot is required for the two-part version, and that is not fussiness.
/// `python313` with no dot is a nixpkgs attribute, appearing inside `nix-shell -p
/// python313` where it names a package and runs nothing. Reading it as a call would
/// invent an interpreter invocation out of a dependency.
pub fn is_python(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("python") else {
        return false;
    };
    match rest.split_once('.') {
        None => {
            rest.is_empty() || rest.len() == 1 && rest.starts_with(|c: char| c.is_ascii_digit())
        }
        Some((major, minor)) => {
            !major.is_empty()
                && major.chars().all(|c| c.is_ascii_digit())
                && !minor.is_empty()
                && minor.chars().all(|c| c.is_ascii_digit())
        }
    }
}

/// What KIND of command this name is, without the flag tables that go with it.
///
/// [`Verb`] is private and must stay so: its variants carry [`Flags`], so making it
/// public emits `private_interfaces`, which under the gate's `-D warnings` is a
/// build failure. This is the payload-free half, for a caller above the file layer
/// that needs a command's kind and has no business with its flag spellings.
///
/// A projection of [`verb`], never a table beside it. A second `match` on the
/// NAME would be a second answer to keep in step by hand, and the first time the two
/// disagreed nothing would say so.
///
/// No `_` arm, deliberately. Adding a [`Verb`] variant is then a compile error
/// here rather than a silent fall into a wrong bucket.
///
/// The strings are a closed vocabulary and are compared by callers, so they are
/// changed the way any wire name is: with everything that reads them.
pub fn verb_kind(name: &str) -> Option<&'static str> {
    Some(match verb(name)? {
        Verb::Read => "read",
        Verb::Search(_) => "search",
        Verb::Stream { .. } => "stream",
        Verb::Remove => "remove",
        Verb::Overwrite => "overwrite",
        Verb::Copy(_) => "copy",
        Verb::Move(_) => "move",
        Verb::Interpreter { .. } => "interpreter",
        Verb::Sql { .. } => "sql",
        Verb::JavaScript => "javascript",
        Verb::Convert { .. } => "convert",
        Verb::Archive => "archive",
        Verb::Python => "python",
        Verb::Script(_) => "script",
        Verb::Carries(_) => "carries",
        Verb::Walk(_) => "walk",
        Verb::Check { .. } => "check",
        Verb::Fetch { .. } => "fetch",
        Verb::ChangeDir => "change directory",
        Verb::Git => "git",
        Verb::Remote(_) => "remote",
        Verb::NoFiles => "no files",
    })
}

/// The one place a command name is read. `None` means "not taught yet", which is
/// [`Op::Unknown`] — never a silent success.
fn verb(name: &str) -> Option<Verb> {
    const SEARCH: Flags = Flags {
        pair: &[],
        pair_file: &[],
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
                pair: &[],
                pair_file: &[],
                valued: &["-e", "-f", "--expression"],
                script: &["-e", "--expression", "-f", "--file"],
                script_file: &["-f", "--file"],
            },
            honours_i: true,
        },
        "awk" | "gawk" => Verb::Stream {
            flags: Flags {
                pair: &[],
                pair_file: &[],
                valued: &["-F", "-v", "-f", "--file"],
                script: &["-f", "--file"],
                script_file: &["-f", "--file"],
            },
            honours_i: false,
        },
        "jq" | "yq" => Verb::Stream {
            flags: Flags {
                // `--arg`/`--argjson` are NAME VALUE — two words, see `Flags::pair`.
                pair: &["--arg", "--argjson"],
                // NAME FILE, and the file is loaded — see `Flags::pair_file`.
                pair_file: &["--slurpfile", "--rawfile", "--argfile"],
                valued: &["-f", "--from-file"],
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

        // A version suffix is part of how this interpreter is spelled, and matching
        // the two bare names missed `python3.12` entirely — not as a wrong reading but as
        // an absence, since a name the table has never heard of produces no `Op::Python`
        // and so appears in no Python report at all. Invisible by construction, which is
        // the reason to match the shape rather than the spellings anyone thought to list.
        name if is_python(name) => Verb::Python,
        // `deno` and `bun` stay [`Verb::Interpreter`] — their `-e` is JavaScript too,
        // but between them they are a handful of calls and neither spells its flags the way
        // node does (`deno eval`, `deno run --allow-read`). Reading them as node would be a
        // guess about a population nobody has measured; they are named here so the absence
        // is a decision and not an oversight.
        "node" => Verb::JavaScript,
        "deno" | "bun" => Verb::Interpreter {
            flags: Flags::valued(&["-e", "-p", "--eval"]),
            inline: &[],
        },
        // The one family whose `-c` really is shell.
        "bash" | "sh" | "zsh" | "dash" | "ksh" => Verb::Interpreter {
            flags: Flags::valued(&["-c", "-o"]),
            inline: &["-c"],
        },
        // `perl -pi -e` is a REWRITER, and reading it as an interpreter got both halves
        // wrong: the file it rewrites was recorded as a script it ran — a read of the
        // wrong direction against a use of the wrong kind. Most of the corpus's perl calls
        // are that one shape.
        //
        // Named beside `sed`, which it is: a program in a flag, files as operands, `-i`
        // deciding whether they are read or rewritten.
        "perl" => Verb::Stream {
            flags: Flags {
                pair: &[],
                pair_file: &[],
                valued: &["-e", "-E", "-I", "-M", "-F"],
                script: &["-e", "-E"],
                script_file: &[],
            },
            honours_i: true,
        },
        // `ruby` stays an interpreter, and that is measured: `ruby -e` appears ZERO
        // times in this corpus. Giving it perl's treatment would be a guess about a
        // population that does not exist.
        "ruby" => Verb::Interpreter {
            flags: Flags::valued(&["-e", "-E", "-I"]),
            inline: &[],
        },
        // Runs the `.lean` file it is given — 177 of its calls name one. The
        // rest are flags and `lake env lean`, where the operand is a target.
        "lean" => Verb::Interpreter {
            flags: Flags::valued(&["--run", "-o", "--o", "-i"]),
            inline: &[],
        },
        "source" | "." => Verb::Interpreter {
            flags: Flags::NONE,
            inline: &[],
        },
        // `sqlite3` was an `Interpreter`, which read its DATABASE as a script it ran —
        // the right file, recorded as the wrong kind of use, and its statements never read
        // at all.
        "sqlite3" => Verb::Sql {
            program: &["-cmd", "-init"],
            file_operand: true,
        },

        // `md5` is the macOS spelling, and the only one that was missing — `md5sum`,
        // `shasum`, `sha1sum`, `sha256sum` and `cksum` have been in the `Verb::Read` list
        // since it was written. Adding "the family" duplicated five of them; the gate's
        // `-D warnings` said so and a local `cargo clippy` did not, because the crate was
        // already built and cargo does not re-emit a warning it has emitted before.
        "md5" | "sha512sum"
        // `openssl x509 -in cert.pem` reads it; this corpus pipes instead
        // (`openssl x509 -noout -enddate`), where the subcommand and its flags
        // are not paths and the guard leaves nothing. Both readings are right.
        | "openssl" => Verb::Read,
        // `wg show wg0 latest-handshakes` — an interface, not a file, in every call here.
        // `wg setconf <file>` and `wg-quick` DO read one, and neither appears; if either
        // starts to, this is where it would go wrong quietly.
        "wg"
        // `ss` is the same shape and was left out only because nothing had measured
        // it. Every spelling in this corpus is flags over a socket table, and the one
        // quoted filter is a socket expression and not a path. Nearly all arrive through
        // `ssh`, which is why they are the fleet's sockets and never this Mac's.
        | "ss" => Verb::NoFiles,

        // Reads every operand, like `cat`. This corpus writes it as `paste - -`,
        // where the operands are stdin and no path is named — but the day one is
        // a file, it is a read, and the path guard already drops the dashes.
        "paste" => Verb::Read,

        // Every call here is real media: the recall pipeline's audio, the heatcam
        // captures. `-i` may be given more than once, and a synthetic input
        // (`-f lavfi -i anoisesrc=duration=2`) is not a path, so the guard drops it without
        // a special case.
        "ffmpeg" => Verb::Convert {
            input: &["-i"],
            output: &[],
        },
        // This repository's own gate is one of these: the Dhall table is the source and
        // `gate.json` is generated from it. Some write by shell redirect instead, which is
        // counted where it stands.
        "dhall-to-json" | "dhall-to-yaml" | "json-to-dhall" | "yaml-to-dhall" => Verb::Convert {
            input: &["--file"],
            output: &["--output"],
        },
        // Reads what it is asked about and writes nothing. Its flag values are
        // `error`, `format=duration`, `csv=p=0` — none of them shaped like a
        // path, so the guard leaves only the file.
        "ffprobe" => Verb::Read,
        "unzip" => Verb::Archive,
        // Some of its calls write a real file — `-X hardcopy /tmp/…` dumps a window's
        // contents, and `-L -Logfile x` logs a session. The rest touch nothing. It was left
        // unread on purpose while the other terminal commands were swept into `NoFiles`,
        // because sweeping it would have deleted those writes silently.
        "screen" => Verb::Fetch {
            writes: &["hardcopy", "-Logfile"],
        },
        // Read-only because that is all this corpus does with it: every call is
        // `zstd -dc <file>`, decompressing to stdout. `zstd <file>` in place would create
        // one and delete the other, and would need its own reading — an undercount if it
        // ever appears, which is the safe side.
        "zstd" | "unzstd" | "zstdcat" => Verb::Read,

        // This fleet's own binary, and its SOURCE is the evidence — the first entry
        // in this table written that way, because the call shapes got it wrong. Calls spell
        // `replay --words <dir>`, `--paper <dir>`, `--bands <dir>`, which reads as valued
        // flags; `scanner/server/src/bin/replay.rs` shows they are bare `flag("--x")` mode
        // tests and the session directory is the only positional. `--page N` is the one
        // flag that takes a value, and left undeclared its `2` resolves against the cwd
        // into a file nothing touched.
        //
        // `Walk` and not `Read` because it is the same shape as `find`: one directory
        // operand, looked *in*. `--pdf` writes, but to stdout.
        "replay" => Verb::Walk(Flags::valued(&["--page"])),
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
        // The JavaScript test runners, high on the unread list. Their operands
        // are spec files and nothing more — neither needs a grammar, which is why
        // they went unread for so long behind the assumption that JavaScript
        // meant a parser. `node -e` really does need one.
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
        // Runs TypeScript the way `node` runs JavaScript, so it gets the same
        // reader: the type annotations this grammar has no rule for land in
        // `stray`, which is what `stray` is for, and the file operations around
        // them read the same either way.
        "tsx" | "ts-node" => Verb::JavaScript,
        // These were `NoFiles`, filed under "build tools that take
        // targets". True of the operand — `mariadb health` is a database name
        // on a server, not a path — and false of everything the command
        // carries: 5,727 corpus commands run a SQL client, and what their
        // statements touched was invisible.
        "mariadb" | "mysql" | "psql" => Verb::Sql {
            program: &["-e", "--execute", "-c", "--command"],
            file_operand: false,
        },
        "cd" => Verb::ChangeDir,
        "git" => Verb::Git,

        // The devshell wrappers. Between them they carry a third of the
        // corpus's commands, and every one was invisible while the shell they
        // open went unread: 15,366 `nix … -c`, 8,870 `nix-shell --run`.
        "nix" => Verb::Carries(&["-c", "--command"]),
        "nix-shell" => Verb::Script(&["--run"]),

        // See [`Verb::Fetch`]: the only local file either one names is the one a
        // flag tells it to save into.
        "curl" => Verb::Fetch {
            writes: &["-o", "--output"],
        },
        "wget" => Verb::Fetch {
            writes: &["-O", "--output-document"],
        },

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
        | "launchctl" | "adb" | "xcrun" | "gh" | "aws" | "rclone"
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
        | "lake"
        // The top of the unread list, each checked against how this corpus
        // actually calls it — `shell-files --show`:
        //
        //   task — the work queue's own CLI. `list`, `show <id>`,
        //     `edit <id> --append`: a store behind a server, and no flag it has
        //     names a file.
        //   ping (`-c 2 -W 2 host`), dig (`+short A name`), nc
        //     (`-z 127.0.0.1 3307`), mariadb-admin (`ping`, `shutdown`) —
        //     network and process, no operand is a path.
        //   journalctl — reads the JOURNAL. `--file` exists and this corpus
        //     never uses it: every call is `-u`, `-b` or `--since`.
        //   dmesg, nixos-version — no operands at all.
        //
        // `screen` is NOT here, and it is the reason to check rather than
        // sweep: a slice of its calls are `-X hardcopy /tmp/…`, which writes a
        // real file. Filing it under this list would have deleted those.
        | "task" | "ping" | "dig" | "nc" | "journalctl" | "dmesg" | "nixos-version"
        | "mariadb-admin"
        // Both measured by `--example unread-shapes`:
        //
        //   mysqladmin — `mariadb-admin` under its old name, every spelling a
        //     `ping` with connection flags. One is
        //     `--socket=/…/mysqld.sock`, a real path this reading discards; it
        //     is safe only because the flag is GLUED, so no operand is left
        //     behind. Written `--socket /path` it would go wrong quietly.
        //   verified_cli — settled from `health/lean/ServeEntry.lean`
        //     rather than from its calls: `cliMain` matches every argument with
        //     `args.contains` against a subcommand name or `--timing`, takes its
        //     data from `IO.getStdin` and returns it on stdout. It opens no file
        //     at all, so the `< legs.json` beside it belongs to the shell, which
        //     this layer reads separately.
        | "mysqladmin" | "verified_cli"
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
    classify_naming(&mut Vec::new(), argv, heredocs, cwd, home)
}

/// As [`classify`], collecting the subjects the text did not determine.
///
/// Two functions rather than one so that the operation and the confession come
/// from the same walk: a caller that wanted both and computed the second itself
/// would need its own copy of the flag tables below, and a second copy is how two
/// answers come to disagree without either being wrong out loud. See [`paths`].
pub fn classify_naming(
    unnamed: &mut Vec<String>,
    argv: &[String],
    heredocs: &[String],
    cwd: Option<&str>,
    home: &str,
) -> Op {
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

    // An INSTALLED program is not a file the agent used, however it was spelled.
    // `/bin/sleep 5` used no file; `./gradlew assembleDebug` used the script in
    // the repo. See [`installed_program`] — the test is the path, not the verb.
    let invoked_by_path = invoked_by_path.filter(|path| !installed_program(path));

    let Some(verb) = verb(name) else {
        return match invoked_by_path {
            Some(script) => Op::Run { script },
            None => Op::Unknown {
                name: name.to_string(),
            },
        };
    };
    match (
        act(unnamed, verb, argv, heredocs, cwd, home),
        invoked_by_path,
    ) {
        (Op::Nothing, Some(script)) => Op::Run { script },
        (op, _) => op,
    }
}

/// Whether this path names a program that was installed rather than a file in
/// the work — a `bin` directory, or a build's output.
///
/// The path, and NOT "is the basename a known verb" — which is what #799
/// proposed and what the corpus refused. `gradlew` is in the verb table, beside
/// `mvn`, `pip` and `ng`, so that rule deleted every `./gradlew` in the fleet —
/// **more reads of a script that lives in the repo than of the noise it was
/// aimed at.** Measured by ablation over the whole corpus of Bash calls.
///
/// This test keeps the two apart because it asks the question that actually
/// distinguishes them: `/nix/store/…/bin/adb` and `.venv/bin/python` are things
/// somebody installed, `android/gradlew` and `picade_fleet/install` are things
/// somebody wrote. A `libexec` component counts — the Android SDK reaches `adb`
/// through one, 232 times.
///
/// It is a guess about a filesystem this reader never sees, and it is allowed
/// to be one *because it only ever withholds a claim*. Mistaking a script for a
/// program loses a use; mistaking a program for a script invents one, and the
/// reader's single forbidden error is recording more than happened.
fn installed_program(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "bin" | "sbin" | ".bin" | "libexec"))
        || path.contains("/target/debug/")
        || path.contains("/target/release/")
}

/// The operation a verb performs on these arguments.
fn act(
    unnamed: &mut Vec<String>,
    verb: Verb,
    argv: &[String],
    heredocs: &[String],
    cwd: Option<&str>,
    home: &str,
) -> Op {
    // A program given by a flag leaves every operand a file, and the file it was
    // given from is itself read.
    let flags = match verb {
        Verb::Search(flags) | Verb::Stream { flags, .. } | Verb::Check { flags, .. } => flags,
        Verb::Interpreter { flags, .. } | Verb::Walk(flags) => flags,
        Verb::Copy(flags) | Verb::Move(flags) => flags,
        // `-c` is the program, `-m` a module, `-W` a warning filter: none of
        // them is an operand, and the first is the whole point.
        // Declared so their values do not become operands — which matters here
        // because the LAST operand is the output.
        // Both ends are flag values here, so both consume the word after them
        // and neither leaves an operand behind.
        Verb::Convert { output, .. } if !output.is_empty() => {
            Flags::valued(&["--file", "--output"])
        }
        Verb::Convert { .. } => Flags::valued(&[
            "-i",
            "-f",
            "-t",
            "-ss",
            "-to",
            "-ar",
            "-ac",
            "-af",
            "-vf",
            "-b:a",
            "-b:v",
            "-c:a",
            "-c:v",
            "-acodec",
            "-vcodec",
            "-map",
            "-filter_complex",
            "-loglevel",
            "-v",
            "-r",
            "-s",
            "-pix_fmt",
            "-frames:v",
        ]),
        Verb::Python => Flags::valued(&["-c", "-m", "-W"]),
        // `-e`/`--eval`/`-p` carry the program; `--input-type` and `-r` carry a
        // mode and a module, and neither is an operand.
        Verb::JavaScript => Flags::valued(&[
            "-e",
            "--eval",
            "-p",
            "--print",
            "--input-type",
            "-r",
            "--require",
        ]),
        Verb::Sql { program, .. } => Flags::valued(program),
        _ => Flags::NONE,
    };
    let from_flag = has_flag(argv, flags.script);
    let script_files = paths(unnamed, &flag_values(argv, flags.script_file), cwd, home);
    let words = operands(argv, &flags);
    // With the program supplied by a flag there is no leading operand to skip.
    let (leading, rest) = match (from_flag, words.split_first()) {
        (false, Some((first, rest))) => ((*first).to_string(), rest),
        _ => (String::new(), &words[..]),
    };

    match verb {
        Verb::Read => Op::Read {
            paths: paths(unnamed, &words, cwd, home),
        },
        Verb::Search(_) => {
            // The file a pattern came from is an argument like any other, so it
            // keeps its place in front of the operands.
            let mut found = script_files;
            found.extend(paths(unnamed, rest, cwd, home));
            Op::Search {
                pattern: leading,
                paths: found,
            }
        }
        Verb::Stream { honours_i, flags } => Op::Transform {
            // A program given by `-e` is still the program. With the text
            // in a flag, `leading` is empty by construction, so this read as a
            // transform with no program at all — every `perl -pi -e` and every
            // `sed -e`, which is 2,664 and 1,410 of them, absent from the
            // opacity census that decides what to read next.
            program: match from_flag {
                true => inline_program(argv, &flags).unwrap_or_default(),
                false => leading,
            },
            program_file: script_files.into_iter().next(),
            // The operands, PLUS the files a `pair_file` flag loaded.
            // `jq --slurpfile a data.json '.f'` reads `data.json`, and it is not
            // an operand — it belongs to the flag. Before `pair_file` existed it
            // survived only because the flag was unmodelled and its two words
            // fell through as operands, which got the file right by accident and
            // the PROGRAM wrong: `a` was recorded as the jq filter.
            paths: {
                let mut all: Vec<&str> = paired_files(argv, flags.pair_file);
                all.extend(rest.iter().copied());
                paths(unnamed, &all, cwd, home)
            },
            // A cluster, not a prefix. `-i` is written `-pi`, `-0pi` and
            // `-ne -i` as often as it is written alone — 2,114 of the corpus's
            // perl calls spell it `-0pi` — and a `starts_with("-i")` test reads
            // every one of them as a command that changed nothing. The same
            // shape `Verb::Remove` already uses for `-r`, and for the same
            // reason: a flag cluster is one word carrying several flags.
            in_place: honours_i && argv.iter().any(|a| spells_in_place(a, flags.valued)),
        },
        Verb::Remove => Op::Remove {
            paths: paths(unnamed, &words, cwd, home),
            recursive: argv
                .iter()
                .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('r'))
                || has_flag(argv, &["--recursive"]),
        },
        Verb::Convert { input, output } if !output.is_empty() => Op::Copy {
            from: paths(unnamed, &flag_values(argv, input), cwd, home),
            to: paths(unnamed, &flag_values(argv, output), cwd, home)
                .into_iter()
                .next()
                .unwrap_or_default(),
        },
        Verb::Convert { input, .. } => {
            let mut from = paths(unnamed, &flag_values(argv, input), cwd, home);
            // Everything positional except the last is another input; ffmpeg
            // accepts none that way, but a file named there is still read.
            let (last, earlier) = match words.split_last() {
                Some((last, earlier)) => (Some(*last), earlier),
                None => (None, &[][..]),
            };
            from.extend(paths(unnamed, earlier, cwd, home));
            match last
                .map(|word| paths(unnamed, &[word], cwd, home))
                .and_then(|found| found.into_iter().next())
            {
                Some(to) => Op::Copy { from, to },
                // No output that is a file — `-f null -`, or a probe. What it
                // read is still what it read.
                None => Op::Read { paths: from },
            }
        }
        Verb::Archive => Op::Read {
            paths: paths(unnamed, &words[..words.len().min(1)], cwd, home),
        },
        Verb::Overwrite => Op::Write {
            paths: paths(unnamed, &words, cwd, home),
        },
        Verb::Copy(_) | Verb::Move(_) => {
            let Some((last, sources)) = words.split_last() else {
                return Op::Nothing;
            };
            let from = paths(unnamed, sources, cwd, home);
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
            Some(rest) if !rest.is_empty() => classify_naming(unnamed, rest, heredocs, cwd, home),
            _ => Op::Nothing,
        },
        Verb::JavaScript => {
            // The program comes from a flag, from a script file, or from stdin —
            // the same three places Python's does, and in the same order.
            if let Some(code) = flag_values(argv, &["-e", "--eval", "-p", "--print"]).first() {
                return Op::JavaScript {
                    source: (*code).to_string(),
                };
            }
            match words
                .first()
                .filter(|word| looks_like_path(word))
                .and_then(|word| resolve(word, cwd, home))
            {
                Some(script) => Op::Run { script },
                None => match heredocs.first() {
                    Some(source) => Op::JavaScript {
                        source: source.clone(),
                    },
                    None => Op::Nothing,
                },
            }
        }
        Verb::Sql {
            program,
            file_operand,
        } => {
            let source = flag_values(argv, program)
                .first()
                .map(|code| (*code).to_string())
                // `mariadb health < dump.sql` and `sqlite3 x.db <<'SQL'` — with
                // no flag carrying them, the statements arrive on stdin.
                .or_else(|| heredocs.first().cloned())
                .unwrap_or_default();
            // Only sqlite3's operand is a path. `looks_like_path` is what keeps
            // `sqlite3 :memory:` and a bare name out of the index.
            let database = if file_operand {
                words
                    .first()
                    .filter(|word| looks_like_path(word))
                    .and_then(|word| resolve(word, cwd, home))
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            if source.is_empty() && database.is_empty() {
                return Op::Nothing;
            }
            Op::Sql { source, database }
        }
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
        // `inline` is empty for `node`, whose `-e` is JavaScript; a non-empty
        // one means the shell family, which is where a flag cluster is a shape
        // the reader has to know about.
        Verb::Interpreter { inline, .. } if !inline.is_empty() && shell_c_value(argv).is_some() => {
            Op::Nested {
                script: shell_c_value(argv).unwrap_or_default().to_string(),
            }
        }
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
            // A destination that cannot be read is still a destination.
            // Filed as an unknown command this would leave the *previous*
            // directory in force, and every relative path after it would
            // resolve against a directory the script had already left.
            Some(word) => Op::ChangeDir {
                to: resolve(word, cwd, home),
            },
            None => Op::ChangeDir {
                to: Some(home.to_string()),
            },
        },
        Verb::Check { writes, .. } => {
            let paths = paths(unnamed, &words, cwd, home);
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
        // Only the saved-into file. The URL is not a path, and the far end is
        // not this machine's — see [`Verb::Fetch`].
        Verb::Fetch { writes } => Op::Write {
            paths: paths(unnamed, &flag_values(argv, writes), cwd, home),
        },
        Verb::Git => git(unnamed, argv, cwd, home),
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
fn git(unnamed: &mut Vec<String>, argv: &[String], cwd: Option<&str>, home: &str) -> Op {
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
            paths: paths(unnamed, &words, base, home),
        }),
        // `rm` deletes, `mv` renames, `restore` overwrites — all real changes.
        // `checkout` is deliberately absent: it takes a branch as often as a
        // path, and `git checkout origin/main` would file a write against a file
        // of that name. It is readable only in its `--` form.
        ("rm" | "restore" | "mv", _) => Op::Git(GitOp::Alter {
            paths: paths(unnamed, &words, base, home),
        }),
        // The separator is the author saying these are paths, which is exactly
        // the guarantee needed.
        (_, Some(at)) => Op::Git(GitOp::Inspect {
            paths: paths(unnamed, &words[at + 1..], base, home),
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

/// The program text a `-e`-style flag carried, if one did.
///
/// A script flag that names a FILE is not this: that file is read, and
/// `script_file` already carries it. Only the flags that hold the program
/// itself.
fn inline_program(argv: &[String], flags: &Flags) -> Option<String> {
    let inline: Vec<&str> = flags
        .script
        .iter()
        .copied()
        .filter(|flag| !flags.script_file.contains(flag))
        .collect();
    flag_values(argv, &inline)
        .first()
        .map(|program| (*program).to_string())
}

/// The machine a command reaches, and the script it hands over.
///
/// Returns [`Op::Nothing`] when there is no script — `ssh host` alone opens a
/// session nobody scripted, and `kubectl get pods` reaches no shell at all.
fn remote(kind: Remote, argv: &[String]) -> Op {
    /// What the far side receives: text a shell will parse, or an argv it will
    /// not. Local to `remote`, because it exists only to carry the difference
    /// the few lines to the `match` below.
    enum Payload {
        Script(String),
        Argv(Vec<String>),
    }
    let (host, payload) = match kind {
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
            (host, Payload::Script(script))
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
            //
            // And when the program is NOT a shell, there is nothing to parse
            // at all — see [`Op::RemoteRun`]. Joining the words and reading the
            // result as shell was where 700 of the nested refusals came from.
            let payload = match script.split_first() {
                // `su - irssi -s /bin/sh -c '…'` names the shell in a flag and
                // the script in another, so the program word is not one of the
                // shells — but a shell is exactly what runs the payload, and
                // reading it as an argv loses the `&&` inside. One payload in
                // the corpus, found by `examples/remote-argv-check.rs` as the
                // single subject the older join-and-parse reading found that
                // this one did not; kept because it is right, not because it is
                // large.
                Some((program, rest)) if matches!(basename(program), "su" | "runuser") => {
                    match rest
                        .iter()
                        .position(|w| w == "-c")
                        .and_then(|i| rest.get(i + 1))
                    {
                        Some(script) => Payload::Script(script.clone()),
                        None => Payload::Argv(script.to_vec()),
                    }
                }
                Some((program, rest))
                    if matches!(basename(program), "sh" | "bash" | "zsh" | "dash" | "ksh") =>
                {
                    let argv: Vec<String> = std::iter::once(program.clone())
                        .chain(rest.iter().cloned())
                        .collect();
                    match shell_c_value(&argv) {
                        Some(script) => Payload::Script(script.to_string()),
                        // `sh script.sh` — a shell with no `-c`, so its words
                        // are still an argv and the first of them is a file.
                        None => Payload::Argv(argv),
                    }
                }
                _ => Payload::Argv(script),
            };
            (target, payload)
        }
    };
    match (host, payload) {
        // A host named by a variable — `ssh "$h" '…'` — cannot be resolved to a
        // machine, and a use filed against `$h` is filed against nothing. The
        // command is dropped rather than attributed to a name that is not one.
        (Some(host), _) if host.contains('$') => Op::Nothing,
        (Some(host), Payload::Script(script)) if !script.trim().is_empty() => Op::Remote {
            host: machine(&host),
            script,
        },
        (Some(host), Payload::Argv(argv)) if !argv.is_empty() => Op::RemoteRun {
            host: machine(&host),
            argv,
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

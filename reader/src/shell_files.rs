//! Which files a shell command used, and which way — as a projection of what
//! the command *does*.
//!
//! [`crate::shell`] reads the syntax and [`crate::shell_ops`] reads the meaning;
//! this is the last step, and it is deliberately the dull one. Each [`Op`]
//! decides its own direction — a `Copy` reads its sources and writes its
//! destination, a `Transform` writes only when it is in place, a `Git::Stage`
//! contributes nothing because staging changes no file — so this is a `match`
//! over the operations rather than a second table to keep in step with the
//! first.
//!
//! What is left here that is not a projection is everything an operation cannot
//! know alone, because it belongs to the *sequence* rather than to any one
//! command: **the working directory**, which `cd` moves and a subshell restores;
//! **what each name is bound to**, which an assignment sets and a second one
//! takes away; and **how many times a body ran**, which only its `for` says. All
//! three need the commands in order, so they live with the loop over them.

use std::collections::BTreeMap;

use crate::shell::Simple;
use crate::shell_ops::{
    GitOp, Op, assignment, basename, classify, expand, looks_like_path, resolve, unwrap_command,
};

/// A file another machine's command used.
///
/// Kept entirely apart from [`FileUse`], and that separation is the point: the
/// path exists on `host`, not here, so it can be reported and counted but must
/// never reach an index of local work. Reading the script is what makes "who
/// maintains isis's nixos config" answerable at all; mixing the two would make
/// every answer about this machine wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUse {
    pub host: String,
    pub path: String,
    pub write: bool,
}

/// A file a command used, absolute, and which way it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUse {
    pub path: String,
    /// `true` when the command changed the file: a `>` redirect, `sed -i`, the
    /// destination of a `cp`, an `rm`. Reads and writes stay apart for the same
    /// reason they do in the tool-call miner — consulting a file and being
    /// responsible for it are different claims.
    pub write: bool,
    /// What had to hold for the command naming this file to run.
    ///
    /// Carried rather than acted on here, because this layer cannot know: the
    /// condition comes from the text and the answer comes from the call's exit
    /// status, which lives with whoever read the transcript. Keeping them apart
    /// is what lets the same extraction serve a report that wants everything and
    /// an index that wants only what certainly happened.
    pub reached: crate::shell::Reached,
}

/// What one script's worth of commands used, with the misses on the record.
#[derive(Debug, Default)]
pub struct Extract {
    pub files: Vec<FileUse>,
    /// Commands whose operation is not known, by name. Not an error — the
    /// honest size of what this does not yet read.
    pub unhandled: BTreeMap<String, usize>,
    /// Commands that were classified, whether or not they named a file.
    pub handled: usize,
    /// Reads and writes by the command that produced them.
    ///
    /// Diagnostic rather than mined: it answers "what is actually doing the
    /// editing", which is the question anyone asks before trusting this. The
    /// answer was not the expected one — `sed` is 10,414 invocations and 96.5%
    /// of them are `sed -n '1,40p' file`, a pager.
    pub by_command: BTreeMap<String, (usize, usize)>,
    /// Every operation, in running order, for callers that want more than the
    /// paths — what was searched for, what was renamed, which scripts ran.
    pub ops: Vec<Op>,
    /// What each of those commands was *doing*, one level up.
    ///
    /// Classified here rather than by the caller because this is the only place
    /// an operation and the command it came from are both in hand: `ops` alone
    /// cannot be zipped against a script's commands, since a nested shell's
    /// operations are absorbed into it and the two lists stop lining up after
    /// the first `bash -c`.
    pub activities: Vec<crate::activity::Activity>,
    /// Files used on another machine, by host. Reported, never mined into the
    /// local index — see [`RemoteUse`].
    pub remote: Vec<RemoteUse>,
    /// What the Python inside the shell did. Its file uses are already in
    /// `files`; this is what could not be read, and is the worklist for
    /// [`crate::python`] exactly as `unhandled` is for the table above.
    pub python: crate::python::Tally,
    /// Commands that exist because a determinate loop was run out — the
    /// difference between the commands a script *wrote* and the ones it *ran*.
    ///
    /// Reported because it moves the denominator under every other figure here:
    /// once loops are unrolled, "simple commands" counts executions, and a
    /// percentage against it is no longer comparable with one from before.
    ///
    /// ⚠ **Subtracting it does not give the commands written.** A `bash -c`
    /// inside a loop body is parsed once per iteration, so the commands *it*
    /// contains are duplicated without any level counting them here.
    pub unrolled: usize,
    /// Nested scripts the grammar could not read. Reported rather than dropped:
    /// a devshell wrapper whose inner shell fails to parse is a silent hole in
    /// exactly the third of the corpus that runs through one.
    pub nested_unparsed: usize,
}

impl Extract {
    /// Record which command produced a use, for the diagnostic tally.
    fn note(&mut self, command: &str, write: bool) {
        let entry = self.by_command.entry(command.to_string()).or_default();
        if write {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }

    /// Record a file use against this machine, or against `host` when the
    /// command is running somewhere else.
    fn push(
        &mut self,
        host: Option<&str>,
        command: &str,
        path: String,
        write: bool,
        reached: crate::shell::Reached,
    ) {
        self.note(command, write);
        match host {
            Some(host) => self.remote.push(RemoteUse {
                host: host.to_string(),
                path,
                write,
            }),
            None => self.files.push(FileUse {
                path,
                write,
                reached,
            }),
        }
    }

    /// Fold a nested script's findings into this one.
    fn absorb(&mut self, inner: Extract) {
        self.files.extend(inner.files);
        self.remote.extend(inner.remote);
        self.ops.extend(inner.ops);
        self.activities.extend(inner.activities);
        self.handled += inner.handled;
        self.unrolled += inner.unrolled;
        self.nested_unparsed += inner.nested_unparsed;
        self.python.merge(inner.python);
        for (name, n) in inner.unhandled {
            *self.unhandled.entry(name).or_insert(0) += n;
        }
        for (name, (r, w)) in inner.by_command {
            let entry = self.by_command.entry(name).or_default();
            entry.0 += r;
            entry.1 += w;
        }
    }
}

/// How deep a `bash -c 'bash -c "…"'` chain is followed.
///
/// The corpus nests twice at most — `nix develop -c bash -c '…'` — so this is a
/// backstop against pathological input rather than a limit anything real meets.
const MAX_NESTING: usize = 4;

/// The files an operation uses, and which way each goes.
///
/// The whole projection, in one place. Every direction here is a property of the
/// operation rather than of a lookup table: a `Move` writes where it lands and
/// reads where it came from, and neither fact needs stating twice.
pub fn files_of(op: &Op, reached: crate::shell::Reached) -> Vec<FileUse> {
    let read = |paths: &Vec<String>| -> Vec<FileUse> {
        paths
            .iter()
            .map(|path| FileUse {
                path: path.clone(),
                write: false,
                reached,
            })
            .collect()
    };
    let write = |paths: &Vec<String>| -> Vec<FileUse> {
        paths
            .iter()
            .map(|path| FileUse {
                path: path.clone(),
                write: true,
                reached,
            })
            .collect()
    };
    match op {
        Op::Read { paths } | Op::Search { paths, .. } => read(paths),
        Op::Write { paths } | Op::Remove { paths, .. } => write(paths),
        Op::Copy { from, to } | Op::Move { from, to } => {
            let mut out = read(from);
            out.push(FileUse {
                path: to.clone(),
                write: true,
                reached,
            });
            out
        }
        Op::Transform {
            program_file,
            paths,
            in_place,
            ..
        } => {
            // The program is read even when the operands are rewritten.
            let mut out: Vec<FileUse> = program_file
                .iter()
                .map(|path| FileUse {
                    path: path.clone(),
                    write: false,
                    reached,
                })
                .collect();
            out.extend(if *in_place { write(paths) } else { read(paths) });
            out
        }
        // A script's own files are collected when it is read, not here — this
        // is one command's operands, and a script is not one. Python is read the
        // same way, in [`extract_nested`], because resolving what it names needs
        // the working directory this function does not have.
        Op::Nested { .. } | Op::Remote { .. } | Op::Python { .. } => Vec::new(),
        Op::Run { script } => vec![FileUse {
            path: script.clone(),
            write: false,
            reached,
        }],
        // **Staging changes no file.** The edit already happened — through
        // `Edit`, `sed` or a redirect — and was counted where it occurred.
        // Counting it again here was 37% of every shell-derived write.
        Op::Git(GitOp::Stage { .. }) => Vec::new(),
        Op::Git(GitOp::Alter { paths }) => write(paths),
        Op::Git(GitOp::Inspect { paths }) => read(paths),
        Op::Git(GitOp::Other { .. }) | Op::ChangeDir { .. } | Op::Nothing | Op::Unknown { .. } => {
            Vec::new()
        }
    }
}

/// Every file the commands of one script used, resolved against `cwd`.
///
/// `cwd` is the directory the `Bash` call ran in — the transcripts record it on
/// every line, and it is the one piece of context a relative path cannot be read
/// without. `None` where it is unknown, in which case only absolute paths
/// survive.
pub fn extract(cmds: &[Simple], cwd: Option<&str>, home: &str) -> Extract {
    extract_nested(cmds, cwd, home, None, 0)
}

/// As [`extract`], tracking how deep inside `bash -c` this script sits and which
/// machine it is running on (`None` for this one).
fn extract_nested(
    cmds: &[Simple],
    cwd: Option<&str>,
    home: &str,
    host: Option<&str>,
    depth: usize,
) -> Extract {
    let mut out = Extract::default();
    // Working directory per subshell scope. A `cd` inside `( … )` writes an
    // entry only that scope and its children can see, so the script it returns
    // to is unaffected — which is what the shell does.
    let mut dirs: BTreeMap<Vec<usize>, Option<String>> = BTreeMap::new();
    dirs.insert(Vec::new(), cwd.map(str::to_string));
    // What each scope has bound, by the same rule as the directory above it: an
    // assignment inside `( … )` is invisible outside it, because the subshell it
    // ran in is gone. `None` is a name that can no longer be trusted: bound
    // twice, or bound to something only running it would answer — see [`bind`].
    let mut binds: BTreeMap<Vec<usize>, BTreeMap<String, Option<String>>> = BTreeMap::new();

    // A loop the text already determines is run out into the commands it ran,
    // before anything looks at any of them — see [`unrolled`].
    let mut ran = unrolled(cmds);
    out.unrolled += ran.len() - cmds.len();
    // ⚠ **Again, now that the iterations exist separately.** The parser demoted
    // the `&&`s the exit status cannot reach, but it saw a loop body *once*. A
    // loop reports only its last iteration's status, so every earlier
    // iteration's `&&` is unconfirmable — and that is only visible after the
    // body has been run out into one copy per value.
    crate::shell::forget_discarded_status(&mut ran);
    let cmds = &ran;
    for cmd in cmds {
        let here = current(&dirs, &cmd.scope);
        // ⚠ **Expansion happens before anything else looks at the words**, so
        // every stage below — the verb table, the path guard, the nested parse —
        // sees the command the shell would have run. A name nobody bound is left
        // written as it was and refused later, exactly as before.
        let env = visible(&binds, &cmd.scope);
        let argv: Vec<String> = cmd.argv.iter().map(|word| expand(word, &env)).collect();
        // A run of `NAME=value` words at the front. Alone they are the whole
        // command and bind the scope; in front of a command they bind for that
        // command only and are gone after it — which is why they are applied to
        // a copy of the environment rather than to the scope.
        let assignments = argv
            .iter()
            .take_while(|word| assignment(word).is_some())
            .count();
        if assignments > 0 && assignments == argv.len() {
            let scope = binds.entry(cmd.scope.clone()).or_default();
            for word in &argv {
                if let Some((name, value)) = assignment(word) {
                    bind(scope, name, value);
                }
            }
            out.handled += 1;
            continue;
        }
        let argv: Vec<String> = if assignments > 0 {
            let mut prefixed = env.clone();
            for word in &argv[..assignments] {
                if let Some((name, value)) = assignment(word)
                    && keepable(value)
                {
                    prefixed.insert(name.to_string(), value.to_string());
                }
            }
            argv[assignments..]
                .iter()
                .map(|word| expand(word, &prefixed))
                .collect()
        } else {
            argv
        };
        let cmd = &Simple {
            argv,
            reached: cmd.reached,
            scope: cmd.scope.clone(),
            redirects: cmd
                .redirects
                .iter()
                .map(|redirect| crate::shell::Redirect {
                    target: expand(&redirect.target, &env),
                    write: redirect.write,
                })
                .collect(),
            heredocs: cmd.heredocs.clone(),
        };
        // A redirect names a file whatever the command is, so it counts even for
        // one that is not understood: `./gradlew build > /tmp/out` is a write.
        let carrier = cmd
            .argv
            .first()
            .map(|head| basename(head).to_string())
            .unwrap_or_else(|| "(redirect)".to_string());
        for redirect in &cmd.redirects {
            if looks_like_path(&redirect.target)
                && let Some(path) = resolve(&redirect.target, here.as_deref(), home)
            {
                out.push(host, &carrier, path, redirect.write, cmd.reached);
            }
        }
        if cmd.argv.is_empty() {
            continue;
        }

        let op = classify(&cmd.argv, &cmd.heredocs, here.as_deref(), home);
        // The name to file this under is the real command's, not the wrapper's.
        let name = unwrap_command(&cmd.argv)
            .first()
            .map(|head| basename(head).to_string())
            .unwrap_or_default();

        match &op {
            // The one operation that changes what comes next rather than
            // producing a file: the scope's own directory moves, and no
            // enclosing one does.
            Op::ChangeDir { to } => {
                dirs.insert(cmd.scope.clone(), to.clone());
                out.handled += 1;
            }
            Op::Unknown { name } => {
                // `cd "$WORKDIR"` — unresolvable, so the directory becomes
                // *unknown*. Leaving it stale would resolve every later relative
                // path somewhere the command never ran.
                if name == "cd" {
                    dirs.insert(cmd.scope.clone(), None);
                }
                *out.unhandled.entry(name.clone()).or_insert(0) += 1;
            }
            // A shell inside a shell: `bash -c '…'`, `nix-shell --run '…'`.
            // Read with the *current* directory and its own scope, so a `cd`
            // inside stays inside — which is what the inner shell does.
            Op::Nested { script } => {
                out.handled += 1;
                match crate::shell::parse(script) {
                    Ok(inner) if depth < MAX_NESTING => {
                        let found = extract_nested(&inner, here.as_deref(), home, host, depth + 1);
                        out.absorb(found);
                    }
                    Ok(_) => {}
                    Err(_) => out.nested_unparsed += 1,
                }
            }
            // Another machine's shell. Read the same way, recorded elsewhere,
            // and with **no working directory**: this one's is meaningless
            // there, so only absolute paths survive unless the script `cd`s
            // first — which many of them do.
            Op::Remote {
                host: there,
                script,
            } => {
                out.handled += 1;
                match crate::shell::parse(script) {
                    Ok(inner) if depth < MAX_NESTING => {
                        let found = extract_nested(&inner, None, home, Some(there), depth + 1);
                        out.absorb(found);
                    }
                    Ok(_) => {}
                    Err(_) => out.nested_unparsed += 1,
                }
            }
            // A program in another language, read by another reader — and
            // resolved here, where the working directory is, by exactly the
            // rules a shell operand goes through.
            Op::Python { source } => {
                out.handled += 1;
                let program = crate::python::read(source);
                let mut kept = 0;
                for used in &program.uses {
                    // A program that moved its own directory makes every
                    // relative path in it a guess, so only the paths that need
                    // no directory survive. `os.chdir` cannot be followed —
                    // its argument is usually computed — and a wrong directory
                    // is how a real path becomes an invented one.
                    let anchored = used.path.starts_with('/') || used.path.starts_with('~');
                    if (anchored || !program.chdir)
                        && looks_like_path(&used.path)
                        && let Some(path) = resolve(&used.path, here.as_deref(), home)
                    {
                        out.push(host, "python", path, used.write, cmd.reached);
                        kept += 1;
                    }
                }
                out.python.kept += kept;
                out.python.absorb(program);
            }
            _ => {
                out.handled += 1;
                for used in files_of(&op, cmd.reached) {
                    out.push(host, &name, used.path, used.write, used.reached);
                }
            }
        }
        out.activities.push(crate::activity::of(&op, cmd));
        out.ops.push(op);
    }
    out
}

/// How far a determinate loop may be run out.
///
/// A backstop against a generated script whose word list is a thousand long
/// turning one body into a thousand commands, not a limit anything real meets:
/// the corpus's longest literal list is well inside it. A loop over the cap is
/// left folded, exactly as every loop was before.
const MAX_UNROLL: usize = 256;

/// Run the loops the text already determines out into the commands they ran.
///
/// ⚠ **This is where the reader stops looking commands up and starts evaluating
/// them.** A `for` over a literal word list says exactly what happened — 4,524
/// of the corpus's 6,474 shell loops — and reading it as a header plus a body
/// full of `$f` threw away the largest single class of subject there is: `$f`
/// alone was refused 1,416 times, `$r` 338, `$d` 308.
///
/// A list is run out only when every value is in the text. A glob is answered by
/// the filesystem of the day, which is gone, and `$(…)` by running something,
/// which never happens here; those loops are left as they were for the path
/// guard to refuse.
///
/// Heredoc bodies are not substituted into. They are data handed to another
/// reader, and a loop variable inside one is rare enough not to be worth the
/// risk of rewriting a program's text.
fn unrolled(cmds: &[Simple]) -> Vec<Simple> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < cmds.len() {
        match run_out(cmds, at) {
            Some((commands, next)) => {
                out.extend(commands);
                at = next;
            }
            None => {
                out.push(cmds[at].clone());
                at += 1;
            }
        }
    }
    out
}

/// One loop run out, with the index just past its `done` — or `None` when the
/// command at `at` does not open a loop the text determines.
///
/// The `for` and its `done` are kept, so the commands a script *wrote* are still
/// all counted; what grows is the body, once per value.
fn run_out(cmds: &[Simple], at: usize) -> Option<(Vec<Simple>, usize)> {
    let (name, values) = literal_loop(&cmds[at].argv)?;
    let end = closing_done(cmds, at)?;
    // Inner loops first, so an outer list multiplies a body already run out.
    let body = unrolled(&cmds[at + 1..end]);
    if values.len().checked_mul(body.len())? > MAX_UNROLL {
        return None;
    }
    let mut out = vec![cmds[at].clone()];
    for value in values {
        let env = BTreeMap::from([(name.to_string(), value.to_string())]);
        out.extend(body.iter().map(|cmd| substituted(cmd, &env)));
    }
    out.push(cmds[end].clone());
    Some((out, end + 1))
}

/// `for NAME in W1 W2 …`, when every value is written out.
///
/// Read through [`unwrap_command`] because a nested loop arrives with the
/// keyword in front of it: `for d in x y; do for f in a b; do …` splits on `;`
/// into a command whose first word is `do`, with the inner `for` behind it.
fn literal_loop(argv: &[String]) -> Option<(&str, Vec<&str>)> {
    let [head, name, over, values @ ..] = unwrap_command(argv) else {
        return None;
    };
    if head != "for" || over != "in" || values.is_empty() {
        return None;
    }
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    values
        .iter()
        .map(|value| determinate(value).then_some(value.as_str()))
        .collect::<Option<Vec<_>>>()
        .map(|values| (name.as_str(), values))
}

/// Whether a word is a value in its own right, needing nothing run and nothing
/// looked up to know it.
fn determinate(word: &str) -> bool {
    !word.is_empty() && !word.contains(['$', '`', '*', '?', '[', '{'])
}

/// The `done` that closes the loop opened at `at`, counting the loops between.
fn closing_done(cmds: &[Simple], at: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (n, cmd) in cmds.iter().enumerate().skip(at) {
        match unwrap_command(&cmd.argv).first().map(String::as_str) {
            Some("for" | "while" | "until" | "select") => depth += 1,
            // `echo done` is not a `done` — this reads the command's own name,
            // which is why the grammar was right to leave the keyword ordinary.
            Some("done") => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(n);
                }
            }
            _ => {}
        }
    }
    None
}

/// One command with the loop's value put in place of its name.
fn substituted(cmd: &Simple, env: &BTreeMap<String, String>) -> Simple {
    Simple {
        argv: cmd.argv.iter().map(|word| expand(word, env)).collect(),
        reached: cmd.reached,
        scope: cmd.scope.clone(),
        redirects: cmd
            .redirects
            .iter()
            .map(|redirect| crate::shell::Redirect {
                target: expand(&redirect.target, env),
                write: redirect.write,
            })
            .collect(),
        heredocs: cmd.heredocs.clone(),
    }
}

/// Whether a value is worth keeping at all, as opposed to being kept as a
/// partial one.
///
/// A value the shell would have had to *run* to know — `$(which adb)`,
/// `` `date` `` — is worth nothing, and keeping its text does active harm: the
/// substitution becomes the command's own name, and the index grows an entry
/// called `$(which adb)`. Only 13 of the corpus's 1,023 `$ADB` uses are this
/// shape, so refusing them costs almost nothing.
///
/// Everything else is kept exactly as written, `$NAME` and all — see [`bind`].
fn keepable(value: &str) -> bool {
    !value.contains("$(") && !value.contains('`')
}

/// Record what a name was bound to, or that it can no longer be trusted.
///
/// A value may be **partly** known: `ADB="$ANDROID_HOME/platform-tools/adb"` is
/// kept whole, with the unexpanded head still in it. Nothing downstream can
/// invent a path from that — [`resolve`] refuses any word still holding a `$` —
/// but `basename` reads `adb`, so the verb table is reachable and the unread
/// list names the tool rather than the variable. That is the whole point: the
/// unknown part of a value must not hide the known part.
///
/// ⚠ **A name bound twice to different values becomes unknown, and stays
/// unknown.** Reading a script top to bottom, "the last assignment wins" looks
/// obvious — but the moment a branch or a loop is involved it is a guess, and
/// this reader takes no branches. `python.rs` drew the same line for the same
/// reason (a name bound exactly once), and two different rules for one idea in
/// one codebase is how they drift apart.
///
/// Bound to the same value twice is not a rebinding. The corpus does it often —
/// the same `ADB=/nix/store/…` in front of several commands — and calling that
/// ambiguous would lose the commonest shape there is.
fn bind(scope: &mut BTreeMap<String, Option<String>>, name: &str, value: &str) {
    if !keepable(value) {
        scope.insert(name.to_string(), None);
        return;
    }
    match scope.get(name) {
        Some(Some(already)) if already == value => {}
        Some(_) => {
            scope.insert(name.to_string(), None);
        }
        None => {
            scope.insert(name.to_string(), Some(value.to_string()));
        }
    }
}

/// Every binding this scope can see, innermost winning — the environment as the
/// shell would have it at this point in the script.
///
/// Only the names that are still trusted: one that was bound twice is absent
/// rather than present-and-wrong, so [`expand`] leaves it written as `$NAME` and
/// the path guard refuses it.
fn visible(
    binds: &BTreeMap<Vec<usize>, BTreeMap<String, Option<String>>>,
    scope: &[usize],
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for n in 0..=scope.len() {
        let Some(here) = binds.get(&scope[..n].to_vec()) else {
            continue;
        };
        for (name, value) in here {
            match value {
                Some(value) => env.insert(name.clone(), value.clone()),
                None => env.remove(name),
            };
        }
    }
    env
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

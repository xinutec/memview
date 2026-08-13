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
    GitOp, Op, assignment, basename, classify_naming, expand, looks_like_path, resolve,
    unwrap_command,
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
    /// Subjects the text does not determine, by the word that stood for them.
    ///
    /// ⚠ **An unknown and an absence are different facts, and this is what keeps
    /// them apart.** A refused word left no trace, so `wc -l "$f"` inside a loop
    /// over a glob recorded what `wc -l` with no operand records — nothing — and
    /// the corpus read as more completely understood than it is. These are the
    /// commands that used a file, said so, and named it with something the
    /// transcript does not contain. See [`crate::shell_ops::undetermined`] for
    /// which refusals qualify and, just as importantly, which do not.
    pub unnamed: BTreeMap<String, usize>,
    /// Nested scripts the grammar could not read. Reported rather than dropped:
    /// a devshell wrapper whose inner shell fails to parse is a silent hole in
    /// exactly the third of the corpus that runs through one.
    pub nested_unparsed: usize,
    /// The walk itself, command by command — empty unless [`trace`] asked for it.
    ///
    /// ⚠ **Recorded by the walk rather than reconstructed from its results**,
    /// and that is the whole reason it exists. Everything else on this struct is
    /// a *total*: the paths, the tallies, the ops. Given only those, anybody
    /// wanting to know why one command attributed one file has to run the walk
    /// again in their own code — with their own idea of expansion, of what a
    /// `cd` did, of which loop was run out — and a second implementation that
    /// disagrees with this one is worse than no view at all, because it disagrees
    /// silently. A step is what this walk saw, at the moment it saw it.
    pub steps: Vec<Step>,
}

/// One command as the walk met it, with everything it decided about it.
///
/// The four layers of [`crate::shell_files`] for a single command, kept together
/// so they can be shown against each other: the words after expansion, the
/// [`Op`] they classified to, the files that projected out, and the
/// [`crate::shell::Reached`] each of those carries. Read on its own, any one of
/// them can look right while the answer is wrong.
#[derive(Debug, Clone)]
pub struct Step {
    /// How many wrappers enclose this command — 0 at the top level, 1 inside the
    /// first `bash -c`, and so on. What a reader sees as indentation.
    pub depth: usize,
    /// The machine it ran on; `None` for this one. Set for every command inside
    /// an `ssh` payload, which is why those steps' uses are in `away` and never
    /// in `files`.
    pub host: Option<String>,
    /// The words **as the shell would have run them**: expansions applied,
    /// leading assignments consumed, a loop's variable replaced by the value
    /// this iteration had. Deliberately not the words as written — the whole
    /// question a reader brings here is why a path came out the way it did, and
    /// the answer is usually something the text does not show.
    pub argv: Vec<String>,
    pub reached: crate::shell::Reached,
    pub scope: Vec<usize>,
    /// The directory its relative paths resolved against, `None` when a `cd`
    /// this reader could not follow made it unknowable.
    pub cwd: Option<String>,
    /// What it classified to — `None` for a line that is a redirection and
    /// nothing else, `> /tmp/log`.
    ///
    /// Absent rather than [`Op::Nothing`], which means something different and
    /// stronger: *this command was understood, and it touches no file*. A line
    /// with no command was never classified at all, and it does write a file.
    pub op: Option<Op>,
    /// The local files this command alone produced, redirects included.
    ///
    /// A wrapper's own step carries only what its *redirect* named: the commands
    /// inside it have steps of their own, and counting their files here too
    /// would show the same use twice at two depths.
    pub files: Vec<FileUse>,
    /// The same for a command that ran on another machine — never mixed into
    /// `files`, for the reason [`RemoteUse`] gives.
    pub away: Vec<RemoteUse>,
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

    /// Begin a step for one command, before anything is decided about it, and
    /// say where it landed so its uses can be attached once they exist.
    ///
    /// `None` where there was no command to begin one for — see [`ran`].
    fn step(
        &mut self,
        op: Option<Op>,
        cmd: &Simple,
        cwd: Option<&str>,
        host: Option<&str>,
        depth: usize,
    ) -> Option<usize> {
        let argv = ran(&cmd.argv);
        // A construct's closing word is not a command and gets no step — unless
        // it carries a redirection, `done < in.txt`, which really does use a
        // file and would otherwise be a use belonging to no step at all.
        if argv.is_empty() && cmd.redirects.is_empty() && cmd.heredocs.is_empty() {
            return None;
        }
        self.steps.push(Step {
            depth,
            host: host.map(str::to_string),
            argv,
            reached: cmd.reached,
            scope: cmd.scope.clone(),
            cwd: cwd.map(str::to_string),
            op,
            files: Vec::new(),
            away: Vec::new(),
        });
        Some(self.steps.len() - 1)
    }

    /// Hand a step the uses that turned out to be its own.
    ///
    /// By range rather than "everything since", because a wrapper's range closes
    /// before the script it opened begins.
    fn attribute(
        &mut self,
        at: usize,
        files: std::ops::Range<usize>,
        away: std::ops::Range<usize>,
    ) {
        let mine = self.files[files].to_vec();
        let theirs = self.remote[away].to_vec();
        let Some(step) = self.steps.get_mut(at) else {
            return;
        };
        step.files.extend(mine);
        step.away.extend(theirs);
    }

    /// Fold a nested script's findings into this one.
    fn absorb(&mut self, inner: Extract) {
        self.files.extend(inner.files);
        self.remote.extend(inner.remote);
        self.ops.extend(inner.ops);
        // After the wrapper's own step, which was pushed before the inner script
        // was read — so a reader scrolling down goes outwards-in, the order the
        // shell itself opens them.
        self.steps.extend(inner.steps);
        self.activities.extend(inner.activities);
        self.handled += inner.handled;
        self.unrolled += inner.unrolled;
        self.nested_unparsed += inner.nested_unparsed;
        self.python.merge(inner.python);
        for (name, n) in inner.unhandled {
            *self.unhandled.entry(name).or_insert(0) += n;
        }
        for (word, n) in inner.unnamed {
            *self.unnamed.entry(word).or_insert(0) += n;
        }
        for (name, (r, w)) in inner.by_command {
            let entry = self.by_command.entry(name).or_default();
            entry.0 += r;
            entry.1 += w;
        }
    }

    /// Every subject a command named and this reader could not, from **both**
    /// readers.
    ///
    /// ⚠ **The fold that was missing, and the headline it moved.** Two readers
    /// keep two accounts — the shell's undetermined words in `unnamed`, Python's
    /// computed paths in `python.unresolved`, and the uses this layer's own rules
    /// turned away in `python.refused` — and for a while only the first was added
    /// up. Python's undetermined subjects outnumber the shell's, so "of all uses"
    /// was a rate over a denominator smaller than it appeared to cover.
    ///
    /// Derived rather than accumulated, deliberately: each account stays in the
    /// one place it is produced, and this is the only thing that adds them
    /// together. A fourth account copied from the other three would drift.
    pub fn subjects_not_named(&self) -> usize {
        self.unnamed.values().sum::<usize>()
            + self.python.unresolved.values().sum::<usize>()
            + self.python.refused.total()
    }
}

/// The words the shell would have run, without the keyword that introduced them.
///
/// ⚠ **`do` never ran.** `for f in a.log; do wc -l "$f"; done` is three commands
/// to this parser, and the body's words arrive as `["do", "wc", "-l", "a.log"]`
/// because `shell.pest` has no rule for a keyword — deliberately, since a rule
/// would have to decide whether `echo done` ends a loop. Classification is
/// unaffected, since everything downstream goes through [`unwrap_command`], but a
/// [`Step`] is the one thing that gets **shown**, and `do wc -l a.log` beside a
/// read of `a.log` reads as a bug in the tool rather than a fact about the work.
///
/// Two kinds of word go, and one deliberately stays:
///
/// * **Introducers** — `if`, `then`, `do`, `!` and the rest. What follows them is
///   a command that ran; they are not.
/// * **Closers** — `done`, `fi`, `esac`. Not commands at all, and nothing
///   follows them, so the step goes with the word.
/// * **A loop or `case` head stays whole.** `for f in *.log` is not a command
///   either, but unlike `done` it carries the one thing worth keeping: the list
///   the loop ran over, which for a folded glob is the only place the pattern
///   appears. Stripping `for` would leave `f in *.log`, which is not anything.
///
/// A wrapper is NOT stripped, and that is the opposite answer to the same
/// question: `sudo rm -rf x` ran as `sudo`, and showing `rm -rf x` would hide
/// how it was run. Leading assignments stay for the same reason — `FOO=bar cmd`
/// changes what the command saw.
fn ran(argv: &[String]) -> Vec<String> {
    const INTRODUCES: [&str; 8] = ["if", "while", "until", "then", "elif", "else", "do", "!"];
    const CLOSES: [&str; 3] = ["done", "fi", "esac"];
    let mut words = argv;
    while let Some(head) = words.first() {
        if CLOSES.contains(&head.as_str()) {
            return Vec::new();
        }
        if !INTRODUCES.contains(&head.as_str()) {
            break;
        }
        words = &words[1..];
    }
    words.to_vec()
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
    extract_nested(cmds, cwd, home, None, 0, false, &[])
}

/// As [`extract`], told which `cd` targets the shell refused.
///
/// **The one thing about a script that the script cannot say.** `cd nope; cat x`
/// reads `x` in the directory it was already in, and nothing in the text says
/// so — the parser applies the move, and every relative path after it is filed
/// under a directory the command never entered. The shell reports the refusal in
/// its output, naming the target; [`crate::doing::refused_dirs`] reads it out.
///
/// A caller with the call's output should use this. `extract` remains for the
/// callers that have none — a corpus row carries a verdict but not the words —
/// and is the same walk with nothing known.
pub fn extract_knowing(
    cmds: &[Simple],
    cwd: Option<&str>,
    home: &str,
    refused: &[String],
) -> Extract {
    extract_nested(cmds, cwd, home, None, 0, false, refused)
}

/// Whether this `cd` is one the shell reported it could not carry out.
///
/// Matched on the target **as written**, because that is what the shell echoes
/// back: `cd: memcheck: No such file or directory` names the word, not the path
/// it would have become. Comparing resolved paths instead would need this layer
/// to reproduce the shell's own expansion of a word it never entered.
///
/// A trailing slash is ignored on both sides — `cd frontend/` is refused as
/// `frontend` — and every operand is tried, so a flagged form like `cd -P dir`
/// is covered without a table of `cd`'s own flags.
fn turned_down(argv: &[String], refused: &[String]) -> bool {
    if refused.is_empty() {
        return false;
    }
    unwrap_command(argv)
        .iter()
        .skip(1)
        .map(|word| word.trim_end_matches('/'))
        .any(|word| refused.iter().any(|target| target == word))
}

/// Whether this `cd` would enter a directory the line says it is already in.
///
/// ⚠ **The rule that makes the transcript's `cwd` usable without settling what
/// it means.** Measured 2026-08-12 over 191,273 `Bash` calls in 40 transcripts:
/// on single-call lines beginning with a relative `cd X`, **168** have the
/// directory the command *started* in and **84** have the one it *ended* in —
/// both readings, in the same transcript, at the same CLI version, on lines that
/// are not rewritten copies. So the field carries both meanings and no property
/// of the line tells them apart (memview #449).
///
/// It does not have to be settled, because one rule is right under both:
///
/// * If `cwd` is where the command ended, its own `cd` is already in the path,
///   and applying it again doubles the segment. All **84** such calls would
///   attribute their files under a directory that has never existed —
///   `…/health/src/src`, `…/health/lean/lean` — checked against the disk.
/// * If `cwd` is where the command started, then `cd X` from inside `X` needs
///   `X/X` to exist, and the shell refused it. A refused `cd` moves nothing.
///
/// Either way the move does not happen. The one shape this gets wrong is a real
/// `cd X` from a directory that genuinely contains `X/X`; no call in the corpus
/// does that — **0** of 135 candidates ever landed in `X/X`, and none of the 84
/// doubled directories exists.
///
/// Relative operands only, and matched as written for the same reason
/// [`turned_down`] matches as written.
fn already_there(argv: &[String], here: Option<&str>) -> bool {
    let Some(here) = here.map(|dir| dir.trim_end_matches('/')) else {
        return false;
    };
    unwrap_command(argv)
        .iter()
        .skip(1)
        .map(|word| word.trim_end_matches('/'))
        .filter(|word| {
            !word.is_empty()
                && !word.starts_with(['/', '~', '$', '-'])
                && !word.split('/').any(|part| part == ".." || part == ".")
        })
        .any(|word| {
            here.len() > word.len()
                && here.ends_with(word)
                && here.as_bytes()[here.len() - word.len() - 1] == b'/'
        })
}

/// As [`extract`], and keeping [`Extract::steps`]: the same walk, saying what it
/// did as it did it.
///
/// ⚠ **A separate entry point rather than the default**, because the corpus runs
/// this 883,000 times in one pass and a step per command is a hundred megabytes
/// nobody asked for. One command's worth is free; every command's is not.
pub fn trace(cmds: &[Simple], cwd: Option<&str>, home: &str) -> Extract {
    extract_nested(cmds, cwd, home, None, 0, true, &[])
}

/// As [`extract`], tracking how deep inside `bash -c` this script sits and which
/// machine it is running on (`None` for this one).
fn extract_nested(
    cmds: &[Simple],
    cwd: Option<&str>,
    home: &str,
    host: Option<&str>,
    depth: usize,
    trace: bool,
    refused: &[String],
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
    // ⚠ **Saturating, because a loop the text determines can run ZERO times.**
    // Every list that could be run out used to have at least one value, so this
    // could only grow and a plain subtraction was safe. `$(seq 3 1)` prints
    // nothing (#821), so its body is dropped and the walk comes back *shorter* —
    // which underflowed here, and in release would have wrapped to a colossal
    // number and taken the whole "from unrolling" figure with it.
    //
    // Nought is the right answer rather than a negative one: this counts commands
    // that exist *because* a loop was run out, and a loop that ran no times
    // brought none into existence. The disappearance is not lost — it is in the
    // total below, which counts what ran.
    out.unrolled += ran.len().saturating_sub(cmds.len());
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
        // Where this command's own uses begin, so the step below can claim
        // exactly them. Taken before the redirects, which are the first thing
        // pushed and belong to the command as much as its operands do.
        let (files_from, away_from) = (out.files.len(), out.remote.len());
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
        // Where the redirects end. A wrapper's step keeps these and nothing
        // after them, because what follows is the inner script's and has steps
        // of its own.
        let redirected = (out.files.len(), out.remote.len());
        if cmd.argv.is_empty() {
            // No command at all — `> /tmp/log` on its own, which the corpus does
            // 8,386 times. It writes a file, so it is worth a step; it
            // classifies to nothing, so it gets no `Op` rather than a borrowed
            // one saying it does nothing with files.
            if trace && let Some(at) = out.step(None, cmd, here.as_deref(), host, depth) {
                out.attribute(at, files_from..redirected.0, away_from..redirected.1);
            }
            continue;
        }

        // ⚠ **The confession comes out of the same walk as the operation**, not
        // from a second pass over the words: which of a command's words were even
        // *subjects* is the flag table's answer, and asking it twice is how two
        // answers come to disagree. See [`crate::shell_ops::classify_naming`].
        let mut unnamed = Vec::new();
        let op = classify_naming(
            &mut unnamed,
            &cmd.argv,
            &cmd.heredocs,
            here.as_deref(),
            home,
        );
        for word in unnamed {
            *out.unnamed.entry(word).or_insert(0) += 1;
        }
        // ⚠ **Pushed before the operation is carried out**, so that a wrapper
        // stands in front of the commands it opens instead of behind them. Its
        // files are attached afterwards, once it is known which of them are this
        // command's own.
        let at = trace
            .then(|| out.step(Some(op.clone()), cmd, here.as_deref(), host, depth))
            .flatten();
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
                // ⚠ **A move the shell refused is not a move.** Everything after
                // `cd nope` in the script ran where it already was, so applying
                // this would file each of its relative paths under a directory
                // that does not exist — `~/Code/memcheck/Cargo.toml` for a
                // `Cargo.toml` read in `~/Code`. Only ever known from the call's
                // own output; see [`crate::doing::refused_dirs`].
                //
                // ⚠ **And a move into where the line already is is not a move
                // either** — see [`already_there`]. That one is not read from
                // the output: it holds whether the shell refused the `cd` or the
                // transcript stamped the directory after it, which is the same
                // rule under both readings of a field that carries both.
                if !turned_down(&cmd.argv, refused) && !already_there(&cmd.argv, here.as_deref()) {
                    dirs.insert(cmd.scope.clone(), to.clone());
                }
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
                        let found = extract_nested(
                            &inner,
                            here.as_deref(),
                            home,
                            host,
                            depth + 1,
                            trace,
                            refused,
                        );
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
                        let found =
                            extract_nested(&inner, None, home, Some(there), depth + 1, trace, &[]);
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
                let mut refused = crate::python::Refused::default();
                for used in &program.uses {
                    // A program that moved its own directory makes every
                    // relative path in it a guess, so only the paths that need
                    // no directory survive. `os.chdir` cannot be followed —
                    // its argument is usually computed — and a wrong directory
                    // is how a real path becomes an invented one.
                    let anchored = used.path.starts_with('/') || used.path.starts_with('~');
                    // ⚠ **Each refusal is recorded, not dropped.** A use turned
                    // away here left no trace at all until memview#824, so a
                    // program that named a file this layer would not resolve
                    // counted exactly as a program that named none — and the
                    // corpus read as more completely understood than it is.
                    if !anchored && program.chdir {
                        refused.moved += 1;
                    } else if !looks_like_path(&used.path) {
                        refused.not_a_path += 1;
                    } else if let Some(path) = resolve(&used.path, here.as_deref(), home) {
                        out.push(host, "python", path, used.write, cmd.reached);
                        kept += 1;
                    } else {
                        refused.no_directory += 1;
                    }
                }
                out.python.kept += kept;
                out.python.refused.merge(&refused);
                out.python.absorb(program);
            }
            _ => {
                out.handled += 1;
                for used in files_of(&op, cmd.reached) {
                    out.push(host, &name, used.path, used.write, used.reached);
                }
            }
        }
        if let Some(at) = at {
            // ⚠ **A wrapper claims its redirects and stops there.** Everything
            // pushed after them came from the script it opened, and those
            // commands have steps of their own — attributing them here as well
            // would show one write twice, once against `nix develop -c …` and
            // once against the `cp` that actually did it.
            let (files_to, away_to) = match op {
                Op::Nested { .. } | Op::Remote { .. } => redirected,
                _ => (out.files.len(), out.remote.len()),
            };
            out.attribute(at, files_from..files_to, away_from..away_to);
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
        let env = BTreeMap::from([(name.to_string(), value)]);
        out.extend(body.iter().map(|cmd| substituted(cmd, &env)));
    }
    out.push(cmds[end].clone());
    Some((out, end + 1))
}

/// `for NAME in W1 W2 …`, when every value is written out — or counted out.
///
/// Read through [`unwrap_command`] because a nested loop arrives with the
/// keyword in front of it: `for d in x y; do for f in a b; do …` splits on `;`
/// into a command whose first word is `do`, with the inner `for` behind it.
fn literal_loop(argv: &[String]) -> Option<(&str, Vec<String>)> {
    let [head, name, over, values @ ..] = unwrap_command(argv) else {
        return None;
    };
    if head != "for" || over != "in" || values.is_empty() {
        return None;
    }
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    // ⚠ **A `$` is not always a question.** `$(seq 1 18)` fails [`determinate`]
    // like any other substitution, and for eighteen months every one of these was
    // left folded on that basis — 1,029 loops, the largest unrun class in the
    // corpus and larger than every glob put together. But nothing about it is
    // unknown: it is arithmetic on numbers already written down, not a question
    // for a filesystem that no longer exists. See [`counted`].
    if let [only] = values
        && let Some(numbers) = counted(only)
    {
        return Some((name.as_str(), numbers));
    }
    values
        .iter()
        .map(|value| determinate(value).then(|| value.to_string()))
        .collect::<Option<Vec<_>>>()
        .map(|values| (name.as_str(), values))
}

/// `$(seq …)` with every bound written out, run out into the numbers it prints.
///
/// ⚠ **This is the reader running a program in its head**, which is a different
/// act from substituting a value it was told, and the list of programs it will do
/// that for is deliberately closed. `seq` is on it because it is 46% of every
/// loop the reader could not run out and because its answer depends on nothing
/// outside the text. Nothing else goes on this list without a measurement saying
/// what it buys.
///
/// A bound that is not a literal integer refuses the whole thing — `$(seq 1
/// $rounds)` is 6 loops in the corpus and belongs with the opaque ones, where
/// what it printed is genuinely gone.
///
/// An empty range is a real answer, not a failure: `seq 3 1` prints nothing, so
/// the body ran zero times, and running it out to nothing is exactly right.
fn counted(word: &str) -> Option<Vec<String>> {
    let inner = word.strip_prefix("$(")?.strip_suffix(')')?.trim();
    let mut words = inner.split_whitespace();
    if words.next()? != "seq" {
        return None;
    }
    let bounds = words
        .map(|bound| bound.parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?;
    // `seq LAST`, `seq FIRST LAST`, `seq FIRST STEP LAST` — the three forms, in
    // the order the tool documents them.
    let (first, step, last) = match bounds[..] {
        [last] => (1, 1, last),
        [first, last] => (first, 1, last),
        [first, step, last] => (first, step, last),
        _ => return None,
    };
    if step == 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut at = first;
    while (step > 0 && at <= last) || (step < 0 && at >= last) {
        // Bounded here as well as in [`run_out`], because that cap is on the
        // commands produced and this one is on the list itself: `seq 1 100000000`
        // must not be built before anybody multiplies it by a body.
        if out.len() >= MAX_UNROLL {
            return None;
        }
        out.push(at.to_string());
        at = at.checked_add(step)?;
    }
    Some(out)
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

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

use crate::project::Ran;
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
    /// Subjects the text does not determine but does **bound**, by the pattern
    /// they are a subset of.
    ///
    /// ⚠ **A glob is not a shrug.** `for f in *.log; do wc -l "$f"; done` names no
    /// file this reader can produce — the directory it was answered against is
    /// gone — but it is not the same fact as `$(git rev-parse HEAD)`, which could
    /// be anything at all. What the text says is
    ///
    /// ```text
    /// ⟦*.log⟧  =  some S  ⊆  L(*.log) ∩ Files(dir, t)
    /// ```
    ///
    /// an unknown finite subset of a **known** language: cardinality unknown,
    /// possibly empty (`nullglob`), possibly the pattern itself where nothing
    /// matched. Recorded as the resolved pattern, so a report can say "some
    /// subset of `/home/example/Code/health/*.log`" rather than "some file".
    ///
    /// Falsifiable, which is the point: run the loop for real and every path it
    /// touches must match the pattern (`S ⊆ L`) — memview#818 does exactly that,
    /// and needs no old filesystem to do it.
    ///
    /// ⚠ These are still **not named**, and [`Extract::subjects_not_named`]
    /// counts them. Bounded is a better answer than opaque; it is not an answer.
    pub bounded: BTreeMap<String, usize>,
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
    /// Subjects this command named that could not be resolved, as written.
    ///
    /// The totals live on [`Extract::unnamed`]; these are the same admissions
    /// attached to the command that made them, because anything *showing* a use
    /// has to show what produced it. A count with no command behind it cannot be
    /// checked by the person reading it.
    pub unnamed: Vec<String>,
    /// And those a glob loop bounded, as the pattern they are a subset of.
    pub bounded: Vec<String>,
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
            unnamed: Vec::new(),
            bounded: Vec::new(),
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
        for (pattern, n) in inner.bounded {
            *self.bounded.entry(pattern).or_insert(0) += n;
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
            + self.bounded.values().sum::<usize>()
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
pub fn extract(ran: &Ran, cwd: Option<&str>, home: &str) -> Extract {
    extract_nested(ran, cwd, home, None, 0, false, &[])
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
pub fn extract_knowing(ran: &Ran, cwd: Option<&str>, home: &str, refused: &[String]) -> Extract {
    extract_nested(ran, cwd, home, None, 0, false, refused)
}

/// As [`extract_knowing`], but recording the walk.
///
/// The combination the effects artefact needs and neither of the others gives:
/// a row shows the *command* that produced a file use, which only a [`Step`]
/// carries, and it must resolve against the directory the shell actually reached,
/// which only the refusals say.
pub fn trace_knowing(ran: &Ran, cwd: Option<&str>, home: &str, refused: &[String]) -> Extract {
    extract_nested(ran, cwd, home, None, 0, true, refused)
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
pub fn trace(ran: &Ran, cwd: Option<&str>, home: &str) -> Extract {
    extract_nested(ran, cwd, home, None, 0, true, &[])
}

/// As [`extract`], tracking how deep inside `bash -c` this script sits and which
/// machine it is running on (`None` for this one).
fn extract_nested(
    ran: &Ran,
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
    // What each scope's glob loops bound, by name — the pattern the variable
    // ranges over, kept apart from `binds` because it is not a value and must
    // never be substituted as one. See [`Extract::bounded`].
    let mut patterns: BTreeMap<Vec<usize>, BTreeMap<String, String>> = BTreeMap::new();

    // ⚠ **The `&&`s a loop's exit status cannot reach were demoted when the
    // loop was run out, not here.** A loop reports only its LAST iteration's
    // status, so every earlier iteration's `&&` is unconfirmable — and that is
    // only visible once the body exists as one copy per value, which is why
    // `project::run_out` applies `forget_discarded_status` after unrolling
    // rather than the parser applying it before.
    out.unrolled += ran.unrolled;
    let cmds = &ran.commands;
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
        // A loop the text cannot run out, but whose values it still bounds. Noted
        // before the body is walked, and never removed at `done`: after the loop
        // the name holds its last value, which is still one of the pattern's
        // matches.
        if let Some((name, values)) = glob_loop(&argv)
            && let [only] = &values[..]
        {
            patterns
                .entry(cmd.scope.clone())
                .or_default()
                .insert(name.to_string(), only.clone());
        }
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
        // ⚠ **A glob loop bounds what its variable ranges over**, so a body that
        // refuses `$f` is not the same admission as one refusing `$(git …)`.
        // Recorded here rather than in the path guard because only the walk knows
        // which loop is standing over this command — the guard sees one command's
        // words and nothing else.
        // ⚠ Every enclosing scope, not just this one — a body that opens a
        // subshell, `for f in *.log; do (wc -l "$f"); done`, sits one level
        // deeper than the loop that bound the name.
        let over = ranging(&patterns, &cmd.scope);
        // Kept beside the totals so the step can carry its own, which is what
        // lets a view show the command an admission came from.
        let (mut refused_here, mut bounded_here) = (Vec::new(), Vec::new());
        for word in unnamed {
            match bounded_by(&word, &over, here.as_deref(), home) {
                Some(pattern) => {
                    *out.bounded.entry(pattern.clone()).or_insert(0) += 1;
                    bounded_here.push(pattern);
                }
                None => {
                    *out.unnamed.entry(word.clone()).or_insert(0) += 1;
                    refused_here.push(word);
                }
            }
        }
        // ⚠ **Pushed before the operation is carried out**, so that a wrapper
        // stands in front of the commands it opens instead of behind them. Its
        // files are attached afterwards, once it is known which of them are this
        // command's own.
        let at = trace
            .then(|| out.step(Some(op.clone()), cmd, here.as_deref(), host, depth))
            .flatten();
        if let Some(at) = at
            && let Some(step) = out.steps.get_mut(at)
        {
            step.unnamed = refused_here;
            step.bounded = bounded_here;
        }
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
                match crate::project::read(script) {
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
                match crate::project::read(script) {
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

/// `for NAME in <pattern>`, where the list is a glob the filesystem answered.
///
/// The complement of [`literal_loop`]: that one runs a loop out because the text
/// determines it, this one recognises the loop it *cannot* run out but can still
/// say something true about. 606 of the corpus's loops are this shape, and the
/// commonest pattern is `*/` — every subdirectory.
///
/// A list mixing a glob with a `$(…)` is refused whole. The glob part is still a
/// bound, but which word the variable took on any iteration is then unknowable,
/// and a bound that only sometimes holds is worse than no bound.
fn glob_loop(argv: &[String]) -> Option<(&str, Vec<String>)> {
    let [head, name, over, values @ ..] = unwrap_command(argv) else {
        return None;
    };
    if head != "for" || over != "in" || values.is_empty() {
        return None;
    }
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    // ⚠ Not [`determinate`], which refuses `*` — that is precisely what makes
    // these loops unrunnable, and testing for it rejects the whole population.
    // What must be absent is an *expansion*: a `$` or a backtick leaves the
    // pattern itself unknown, and a brace expands one word into several, so the
    // variable ranges over a list this does not hold.
    if values
        .iter()
        .any(|value| value.contains(['$', '`', '{']) || value.is_empty())
    {
        return None;
    }
    values
        .iter()
        .any(|value| value.contains(['*', '?', '[']))
        .then(|| (name.as_str(), values.to_vec()))
}

/// Every pattern a glob loop bound that this scope can see, innermost winning.
///
/// The counterpart of [`visible`] for values, and separate from it for the
/// reason [`Extract::bounded`] gives: a pattern is not a value and must never be
/// substituted as one.
fn ranging(
    patterns: &BTreeMap<Vec<usize>, BTreeMap<String, String>>,
    scope: &[usize],
) -> BTreeMap<String, String> {
    let mut over = BTreeMap::new();
    for n in 0..=scope.len() {
        let Some(here) = patterns.get(&scope[..n].to_vec()) else {
            continue;
        };
        for (name, pattern) in here {
            over.insert(name.clone(), pattern.clone());
        }
    }
    over
}

/// The pattern a refused word is a subset of, if a glob loop bound its name.
///
/// ⚠ **Only a plain substitution keeps the bound.** `$f` and `$f/package.json`
/// are the pattern and the pattern concatenated with a literal — both regular,
/// both honestly stateable. `${f%%:*}` is a *rational transduction* of it, which
/// needs the automaton this deliberately does not build, so it stays opaque. The
/// test for that is what follows the name: `}` keeps the bound, an operator does
/// not.
fn bounded_by(
    word: &str,
    patterns: &BTreeMap<String, String>,
    cwd: Option<&str>,
    home: &str,
) -> Option<String> {
    for (name, pattern) in patterns {
        let plain = format!("${name}");
        let braced = format!("${{{name}}}");
        let put = if word.contains(&braced) {
            word.replace(&braced, pattern)
        } else if let Some(at) = word.find(&plain) {
            // `$f` must not match the start of `$file`, and `${f%%:*}` is caught
            // by the braced test above failing.
            let after = word[at + plain.len()..].chars().next();
            if after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            word.replace(&plain, pattern)
        } else {
            continue;
        };
        if put.contains('$') {
            // A second name this did not resolve. Bounded by one thing and
            // unknown in another is not bounded.
            continue;
        }
        return resolve(&put, cwd, home);
    }
    None
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

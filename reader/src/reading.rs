//! What the table read out of a whole corpus, as a value rather than as print.
//!
//! `--bin shell-files` was the only thing that knew how to survey a corpus, and
//! it knew it as thirty mutable locals inside `main`. That is fine for a report
//! nobody else reads, and it is exactly wrong the moment a *second* consumer
//! wants the same numbers: the alternative to this module is the API recomputing
//! the survey its own way, and two answers to "how much is understood" that
//! drift apart silently.
//!
//! So the accumulation lives here and the binary prints from it. **The report is
//! now a view of this type, not a separate calculation** — which is the only
//! form in which "the number on the phone is the number in the report" is a fact
//! rather than a hope.
//!
//! ⚠ **Counted per call, not per distinct command.** Forty runs of one command
//! count forty times, because frequency is the whole signal here: a command run
//! four thousand times is worth adding to the table and one run once is not.
//! `shell-report` counts the other way, and says so for the same reason.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::shell_files;
use crate::shell_ops::{GitOp, Op};

/// How one operation is named, in the three registers the views need.
///
/// ⚠ **One definition, because two drifted.** The console labelled a chip and
/// the viewer labelled a histogram row, each from its own exhaustive `match`.
/// Both compile when an `Op` variant is added — the compiler forces a value,
/// not a consistent one — so the same command could be called two different
/// things in two places, and was: `Op::Nothing` was `nothing` on the phone and
/// `nothing with files` in the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Naming {
    /// A stable key for styling and data attributes.
    ///
    /// ⚠ **Never displayed, so wording can change without breaking CSS.** The
    /// console's chip colours select on this — `[data-kind='unknown']` — and
    /// when the key WAS the display string, improving a label silently dropped
    /// its colour.
    pub key: &'static str,
    /// One or two words, for a chip. A phone is 412px and the chip shares its
    /// line with a host and a depth note.
    pub chip: &'static str,
    /// A phrase, for a row in a distribution, read by somebody asking what the
    /// fleet *does* rather than by somebody holding the enum.
    pub phrase: &'static str,
}

const fn name(key: &'static str, chip: &'static str, phrase: &'static str) -> Naming {
    Naming { key, chip, phrase }
}

/// What to call an operation.
pub fn naming(op: &Op) -> Naming {
    match op {
        Op::Read { .. } => name("read", "read", "read"),
        Op::Write { .. } => name("write", "write", "write"),
        Op::Remove { .. } => name("remove", "remove", "remove"),
        Op::Copy { .. } => name("copy", "copy", "copy"),
        Op::Move { .. } => name("move", "move", "move"),
        Op::Search { .. } => name("search", "search", "search"),
        Op::Transform { in_place: true, .. } => {
            name("transform", "rewrite", "transform (in place)")
        }
        Op::Transform { .. } => name("transform", "transform", "transform"),
        Op::Run { .. } => name("run", "run a script", "run a script"),
        Op::Nested { .. } => name(
            "shell",
            "opens a shell",
            "open a shell (bash -c, nix --run)",
        ),
        Op::Python { .. } => name("python", "python", "run python (-c, or a heredoc)"),
        Op::JavaScript { .. } => name(
            "javascript",
            "javascript",
            "run javascript (-e, or a heredoc)",
        ),
        Op::Sql { .. } => name("sql", "sql", "query a database"),
        Op::Remote { .. } => name(
            "remote",
            "elsewhere",
            "reach another machine (ssh, kubectl exec)",
        ),
        Op::RemoteRun { .. } => name(
            "remote",
            "elsewhere",
            "run a program on another machine (no shell)",
        ),
        Op::ChangeDir { .. } => name("cd", "cd", "cd"),
        Op::Git(GitOp::Stage { .. }) => name("git", "git", "git stage"),
        Op::Git(GitOp::Alter { .. }) => name("git", "git", "git alter"),
        Op::Git(GitOp::Inspect { .. }) => name("git", "git", "git inspect"),
        Op::Git(GitOp::Other { .. }) => name("git", "git", "git (other)"),
        // ⚠ **"nothing" alone is FALSE and was on screen.** It means the command
        // touched no files; on a chip beside `ping`, `task list` or
        // `ssh host uptime` it reads as "this command did nothing". The word
        // that carries the meaning is the one the chip had dropped.
        Op::Nothing => name("nothing", "no files", "nothing with files"),
        Op::Unknown { .. } => name("unknown", "not read", "not understood"),
    }
}

/// The shape of an operation, for the distribution.
pub fn op_name(op: &Op) -> &'static str {
    naming(op).phrase
}

/// One corpus row, as much of it as the survey needs.
///
/// Taken as a borrowed struct rather than a `serde_json::Value` so that a caller
/// holding rows in some other shape — the console holds them as its own type —
/// does not have to build JSON to be surveyed.
pub struct Row<'a> {
    pub cmd: &'a str,
    pub cwd: Option<&'a str>,
    /// The `cd` targets the shell refused, which only its own output knows.
    pub refused: &'a [String],
    /// What became of the call.
    ///
    /// ⚠ **Absence is `Unknown`, never success.** A corpus written before
    /// outcomes were recorded has no such field, and reading that silence as
    /// `Ok` would attribute every file use in it to somebody.
    pub ran: crate::doing::Verdict,
}

/// Everything the survey accumulates.
///
/// Public fields, deliberately: this is a tally, and a wall of getters over a
/// tally is ceremony. The invariant that matters is not encapsulation but that
/// [`Reading::absorb`] is the only thing that writes them.
#[derive(Debug, Default)]
pub struct Reading {
    pub calls: usize,
    pub unparsed: usize,
    pub handled: usize,
    pub unhandled: usize,
    pub unrolled: usize,
    /// Commands the table has no entry for, by name — the work queue.
    pub by_name: BTreeMap<String, usize>,
    /// Calls to a function the calling script declares, and their names.
    ///
    /// ⚠ **Its own line on every view, and it is NOT in `understood()`.** These
    /// are not commands anybody can teach the table — see
    /// [`crate::shell_files::Extract::local`] — but the reader did not follow
    /// the call either, so a rate that counted them would say it understands
    /// something it has not read.
    pub local: usize,
    pub local_by_name: BTreeMap<String, usize>,
    /// Calls whose command NAME is a variable nobody bound, and their names.
    ///
    /// ⚠ **The same standing as `local`: in `commands()`, not in
    /// `understood()`.** `$BIN` names a different program in every script, so
    /// there is no entry to teach — and nothing was read either, so a rate that
    /// counted it would claim an understanding it does not have. See
    /// [`crate::shell_files::Extract::from_a_variable`].
    pub from_a_variable: usize,
    pub from_a_variable_by_name: BTreeMap<String, usize>,
    /// Subjects the text does not determine. Counted beside the commands not
    /// read at all, because they are the same kind of admission: the size of
    /// what this does not know, stated by the thing that does not know it.
    pub unnamed: usize,
    pub by_word: BTreeMap<String, usize>,
    /// Subjects a glob loop BOUNDED — still unnamed, but a subset of a pattern
    /// rather than of anything at all.
    pub by_pattern: BTreeMap<String, usize>,
    /// Subjects with a LOCUS but no language, by the directory they are rooted
    /// at — the other half of the same object as `by_pattern`, for the words a
    /// glob never bound.
    pub by_locus: BTreeMap<String, usize>,
    /// The same admission from the Python reader: the call that wanted a path,
    /// where the path was computed and no text of it survives.
    pub computed: BTreeMap<String, usize>,
    /// Python operations whose path is one of a known finite set, by the set.
    ///
    /// ⚠ **Counted by `subjects_not_named`, exactly as `computed` is.** These
    /// moved out of `computed` when the reader learned to keep a name's several
    /// literal bindings, and if the total had fallen by that many it would have
    /// been reporting a denominator change as knowledge.
    pub python_bounded: BTreeMap<String, usize>,
    /// The JavaScript reader's share of the same admission.
    ///
    /// ⚠ **`subjects_not_named` has always counted these and this breakdown has
    /// never shown them**, so the lines under the headline summed to 489 less
    /// than the headline and a reader adding them up would find the table
    /// short. Listed rather than folded into another line: it is a different
    /// reader, and merging it would hide which one the work is in.
    pub javascript_unnamed: usize,
    /// And the uses Python DID name that this layer's own rules turned away.
    pub turned_away: crate::python::Refused,
    /// What the SQL touched — **tables, never files**. See
    /// [`crate::shell_files::Extract::tables`] for why the two are kept apart.
    pub tables: crate::sql::Queried,
    pub reads: usize,
    pub writes: usize,
    /// Distinct paths, so the size of what this produces is visible rather than
    /// implied by a total that double-counts every file opened twice.
    pub distinct: BTreeMap<String, (usize, usize)>,
    /// Which commands actually open and change files — the check anyone runs
    /// before trusting the table, and the one that showed `sed` to be a pager.
    pub by_command: BTreeMap<String, (usize, usize)>,
    pub by_op: BTreeMap<&'static str, usize>,
    pub searched: BTreeMap<String, usize>,
    pub renames: usize,
    pub nested_unparsed: BTreeMap<String, usize>,
    /// File uses by what had to hold for the command naming them to run.
    pub always: usize,
    pub on_success: usize,
    pub sometimes: usize,
    /// Those the call's outcome confirms actually happened.
    pub certain: usize,
    /// What happens on the other machines — read from the same scripts, kept out
    /// of every local figure.
    pub remote: BTreeMap<String, (usize, usize)>,
    pub remote_paths: BTreeMap<(String, String), (usize, usize)>,
    /// A path substring to collect the evidence for, and how much of it.
    ///
    /// ⚠ **This is the check that matters, and it lives here rather than in the
    /// report for that reason.** Not how many paths came out, but whether a
    /// given one came from a command that really names it — every doubt about
    /// this table has been settled by reading the commands behind one suspicious
    /// path, and a survey that could not answer it would send the next person
    /// back to grep.
    watch: Option<(String, usize)>,
    /// What `watch` caught: whether it was a write, the path, and the command.
    pub witnesses: Vec<(bool, String, String)>,
}

impl Reading {
    /// A survey that also collects the commands behind paths matching `why`.
    pub fn watching(why: &str, limit: usize) -> Reading {
        Reading {
            watch: Some((why.to_string(), limit)),
            ..Reading::default()
        }
    }

    /// Read one call into the tally.
    ///
    /// `home` is what `~` expands to. It is a parameter rather than read from
    /// the environment here so that a survey of somebody else's corpus is
    /// possible at all, and so tests need not set a variable.
    pub fn absorb(&mut self, row: &Row<'_>, home: &str) {
        self.calls += 1;
        let Ok(parsed) = crate::project::read(row.cmd) else {
            self.unparsed += 1;
            return;
        };
        let found = shell_files::extract_knowing(&parsed, row.cwd, home, row.refused);
        self.handled += found.handled;
        for (reason, n) in &found.nested_unparsed {
            *self.nested_unparsed.entry(reason.clone()).or_insert(0) += n;
        }
        self.unrolled += found.unrolled;
        for use_ in &found.remote {
            let host = self.remote.entry(use_.host.clone()).or_default();
            let path = self
                .remote_paths
                .entry((use_.host.clone(), use_.path.clone()))
                .or_default();
            if use_.write {
                host.1 += 1;
                path.1 += 1;
            } else {
                host.0 += 1;
                path.0 += 1;
            }
        }
        for (name, (r, w)) in &found.by_command {
            let entry = self.by_command.entry(name.clone()).or_default();
            entry.0 += r;
            entry.1 += w;
        }
        for op in &found.ops {
            *self.by_op.entry(op_name(op)).or_insert(0) += 1;
            match op {
                Op::Search { pattern, .. } if !pattern.is_empty() => {
                    *self.searched.entry(pattern.clone()).or_insert(0) += 1;
                }
                Op::Move { .. } => self.renames += 1,
                _ => {}
            }
        }
        for (name, n) in &found.unhandled {
            self.unhandled += n;
            *self.by_name.entry(name.clone()).or_insert(0) += n;
        }
        for (name, n) in &found.local {
            self.local += n;
            *self.local_by_name.entry(name.clone()).or_insert(0) += n;
        }
        for (name, n) in &found.from_a_variable {
            self.from_a_variable += n;
            *self
                .from_a_variable_by_name
                .entry(name.clone())
                .or_insert(0) += n;
        }
        // ⚠ The total comes from `subjects_not_named`, which folds in the Python
        // and JavaScript readers' accounts too — the map below is the shell's
        // words alone, and reading a total off it is the undercount memview#824
        // was about.
        self.unnamed += found.subjects_not_named();
        self.javascript_unnamed +=
            found.javascript.unresolved.values().sum::<usize>() + found.javascript.refused.total();
        for (word, n) in &found.unnamed {
            *self.by_word.entry(word.clone()).or_insert(0) += n;
        }
        for (pattern, n) in &found.bounded {
            *self.by_pattern.entry(pattern.clone()).or_insert(0) += n;
        }
        for (dir, n) in &found.located {
            *self.by_locus.entry(dir.clone()).or_insert(0) += n;
        }
        for (call, n) in &found.python.unresolved {
            *self.computed.entry(call.clone()).or_insert(0) += n;
        }
        for (set, n) in &found.python.bounded {
            *self.python_bounded.entry(set.clone()).or_insert(0) += n;
        }
        self.turned_away.merge(&found.python.refused);
        self.tables.merge(&found.tables);
        for file in found.files {
            if let Some((why, limit)) = &self.watch
                && file.path.contains(why.as_str())
                && self.witnesses.len() < *limit
            {
                self.witnesses
                    .push((file.write, file.path.clone(), row.cmd.to_string()));
            }
            match file.reached {
                crate::shell::Reached::Always => self.always += 1,
                crate::shell::Reached::OnSuccess => self.on_success += 1,
                crate::shell::Reached::Sometimes => self.sometimes += 1,
            }
            if row.ran.admits(file.reached) {
                self.certain += 1;
            }
            let entry = self.distinct.entry(file.path).or_default();
            if file.write {
                self.writes += 1;
                entry.1 += 1;
            } else {
                self.reads += 1;
                entry.0 += 1;
            }
        }
    }

    /// Commands *run*, not commands written: a determinate loop is run out into
    /// its iterations before this counts them.
    pub fn commands(&self) -> usize {
        self.handled + self.unhandled + self.local + self.from_a_variable
    }

    /// How much of what ran the table has an entry for.
    ///
    /// ⚠ **`local` is in the DENOMINATOR and not in the numerator, and that is
    /// the whole point of splitting it out.** Moving 2,493 calls from
    /// `unhandled` into their own bucket must not raise this rate: nothing more
    /// was read, and a coverage figure that improves because calls left the
    /// denominator is a different fact from one that improves because the reader
    /// learned something (\[\[feedback_a_threshold_carries_its_denominator\]\]).
    /// Measured across the union corpus, this refactor left it at 99.3%.
    pub fn understood(&self) -> f64 {
        100.0 * self.handled as f64 / self.commands().max(1) as f64
    }

    /// The share of file uses whose subject the text does not determine.
    ///
    /// ⚠ **Stated as a rate against the uses, not left as a bare count.**
    /// Without it, `distinct` reads as "every file that was used", which is the
    /// overstatement this number exists to end.
    pub fn opaque(&self) -> f64 {
        100.0 * self.unnamed as f64 / (self.reads + self.writes + self.unnamed).max(1) as f64
    }

    /// Read a `.jsonl` corpus in the shape `bash-corpus` writes.
    ///
    /// ⚠ **A row whose outcome is present and UNREADABLE is an error**, not an
    /// `Unknown`. Quietly downgrading it would turn a corrupt corpus into a
    /// modest-looking one.
    pub fn of_corpus(text: &str, home: &str) -> anyhow::Result<Reading> {
        Reading::default().read_corpus(text, home)
    }

    /// The same, also collecting the commands behind paths matching `why`.
    pub fn of_corpus_watching(
        text: &str,
        home: &str,
        why: &str,
        limit: usize,
    ) -> anyhow::Result<Reading> {
        Reading::watching(why, limit).read_corpus(text, home)
    }

    fn read_corpus(mut self, text: &str, home: &str) -> anyhow::Result<Reading> {
        let reading = &mut self;
        for line in text.lines() {
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(cmd) = row["cmd"].as_str() else {
                continue;
            };
            let refused: Vec<String> = row["refused"]
                .as_array()
                .map(|it| {
                    it.iter()
                        .filter_map(|t| t.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let ran: crate::doing::Verdict = match row.get("ran") {
                None | Some(serde_json::Value::Null) => crate::doing::Verdict::Unknown,
                Some(outcome) => serde_json::from_value(outcome.clone())
                    .map_err(|_| anyhow::anyhow!("unreadable outcome in the corpus: {outcome}"))?,
            };
            reading.absorb(
                &Row {
                    cmd,
                    cwd: row["cwd"].as_str().filter(|c| !c.is_empty()),
                    refused: &refused,
                    ran,
                },
                home,
            );
        }
        Ok(self)
    }
}

/// One row of a ranked table, for the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ranked {
    pub name: String,
    pub n: usize,
}

/// A path or a command with both directions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Both {
    pub name: String,
    pub reads: usize,
    pub writes: usize,
}

/// The compact view an API serves.
///
/// ⚠ **Named for what it is rather than `Summary`, because the wire-mirror check
/// matches TypeScript to Rust BY NAME.** `Reading` and `Summary` are both taken
/// by unrelated wire types in this workspace — a subscription usage reading, and
/// a session summary — and a mirror that resolves to the wrong struct reports
/// every field of this one as drift, which is exactly what it did.
///
/// ⚠ **Every list is truncated and the totals are not.** A summary that ranked
/// the top ten and reported ten as the total would be a lie by omission of
/// exactly the kind this codebase keeps finding; so the counts above the lists
/// are over everything, and the lists say how far down they go by being lists.
///
/// The full detail stays in `--bin shell-files`, which prints from the same
/// [`Reading`]. This is what fits on a phone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorpusRead {
    /// When the corpus this was read from was last written, in epoch
    /// seconds. Not formatted here — see `--bin reading-json`.
    pub corpus_at: Option<i64>,
    pub calls: usize,
    pub unparsed: usize,
    pub commands: usize,
    pub unrolled: usize,
    pub handled: usize,
    pub unhandled: usize,
    /// Calls to a function the calling script declares.
    ///
    /// ⚠ **In `commands` and not in `handled`**, so `understood` is unmoved by
    /// splitting this out — nothing more was read. See [`Reading::understood`].
    pub local: usize,
    /// Calls whose command name is a variable nobody bound.
    ///
    /// ⚠ **In `commands` and not in `handled`**, exactly as `local` is, and for
    /// the same reason: nothing more was read.
    pub from_a_variable: usize,
    /// `handled` as a percentage of `commands`, computed once so two clients
    /// cannot round it two ways.
    pub understood: f64,
    pub reads: usize,
    pub writes: usize,
    pub distinct_paths: usize,
    pub always: usize,
    pub on_success: usize,
    pub sometimes: usize,
    pub certain: usize,
    pub unnamed: usize,
    /// `unnamed` as a percentage of all uses including it.
    pub opaque: f64,
    pub unnamed_by_word: usize,
    pub unnamed_bounded: usize,
    /// Subjects with a locus but no language — see `Extract::located`.
    pub unnamed_located: usize,
    pub unnamed_computed: usize,
    /// Python operations whose path is one of a known finite set — see
    /// `Reading::python_bounded`. Its own field because it moved OUT of
    /// `unnamed_computed`, and a client showing only the old one would report a
    /// fall of 2,719 that was a reclassification rather than a naming.
    pub unnamed_python_set: usize,
    /// The JavaScript reader's share, which `unnamed` has always included and
    /// no client has ever shown.
    pub unnamed_javascript: usize,
    pub refused_here: usize,
    /// Table reads and changes, and how many distinct tables that is.
    ///
    /// ⚠ **Beside the file counts and never inside them.** A table is not a
    /// file: 2,747 table reads added to `reads` would be 2,747 files that do not
    /// exist. Measured — no statement in this corpus names a file at all.
    pub table_reads: usize,
    pub table_writes: usize,
    pub distinct_tables: usize,
    /// The busiest tables, read and changed.
    pub tables: Vec<Both>,
    /// What the SQL was doing, by verb.
    pub sql: Vec<Ranked>,
    /// What the shell was doing, biggest first. Not truncated: there are
    /// twenty-two shapes in total and the tail is the interesting part.
    pub doing: Vec<Ranked>,
    pub renames: usize,
    pub busiest: Vec<Both>,
    pub writers: Vec<Both>,
    pub hosts: Vec<Both>,
    pub unread: Vec<Ranked>,
    /// The local functions, biggest first — the same shape as `unread` and
    /// deliberately a separate list, because one is a worklist and the other
    /// can never be worked.
    pub local_names: Vec<Ranked>,
    /// The variable-named calls, biggest first. Beside `local_names` and for
    /// the same reason: neither list can ever be worked.
    pub variable_names: Vec<Ranked>,
    pub opaque_words: Vec<Ranked>,
}

/// How far down each ranked list in a [`CorpusRead`] goes.
///
/// One constant rather than a parameter: the number is a property of the screen
/// the summary is drawn on, and a caller choosing it per-list is a caller who
/// will eventually choose two different ones.
const DEEP: usize = 12;

fn rank(map: &BTreeMap<String, usize>) -> Vec<Ranked> {
    let mut all: Vec<_> = map.iter().collect();
    all.sort_by_key(|(name, n)| (std::cmp::Reverse(**n), (*name).clone()));
    all.into_iter()
        .take(DEEP)
        .map(|(name, n)| Ranked {
            name: name.clone(),
            n: *n,
        })
        .collect()
}

/// The busiest tables, folding the two directions into one row each.
///
/// Ranked by total traffic rather than by writes: a table read three hundred
/// times is the centre of the work whether or not anything changed it, and a
/// list ordered by writes would put a table touched twice above it.
fn busiest_tables(queried: &crate::sql::Queried) -> Vec<Both> {
    let mut merged: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (name, n) in &queried.reads {
        merged.entry(name).or_default().0 += n;
    }
    for (name, n) in &queried.writes {
        merged.entry(name).or_default().1 += n;
    }
    let mut all: Vec<_> = merged.into_iter().collect();
    all.sort_by_key(|(name, (r, w))| (std::cmp::Reverse(r + w), *name));
    all.into_iter()
        .take(DEEP)
        .map(|(name, (r, w))| Both {
            name: name.to_string(),
            reads: r,
            writes: w,
        })
        .collect()
}

fn rank_both(map: &BTreeMap<String, (usize, usize)>, writing_only: bool) -> Vec<Both> {
    let mut all: Vec<_> = map
        .iter()
        .filter(|(_, (_, w))| !writing_only || *w > 0)
        .collect();
    all.sort_by_key(|(name, (r, w))| {
        let by = if writing_only { *w } else { r + w };
        (std::cmp::Reverse(by), (*name).clone())
    });
    all.into_iter()
        .take(DEEP)
        .map(|(name, (r, w))| Both {
            name: name.clone(),
            reads: *r,
            writes: *w,
        })
        .collect()
}

impl Reading {
    pub fn summary(&self, corpus_at: Option<i64>) -> CorpusRead {
        let mut doing: Vec<_> = self
            .by_op
            .iter()
            .map(|(name, n)| Ranked {
                name: (*name).to_string(),
                n: *n,
            })
            .collect();
        doing.sort_by(|a, b| b.n.cmp(&a.n).then_with(|| a.name.cmp(&b.name)));
        CorpusRead {
            corpus_at,
            calls: self.calls,
            unparsed: self.unparsed,
            commands: self.commands(),
            unrolled: self.unrolled,
            handled: self.handled,
            unhandled: self.unhandled,
            local: self.local,
            from_a_variable: self.from_a_variable,
            understood: self.understood(),
            reads: self.reads,
            writes: self.writes,
            distinct_paths: self.distinct.len(),
            always: self.always,
            on_success: self.on_success,
            sometimes: self.sometimes,
            certain: self.certain,
            unnamed: self.unnamed,
            opaque: self.opaque(),
            unnamed_by_word: self.by_word.values().sum(),
            unnamed_bounded: self.by_pattern.values().sum(),
            unnamed_located: self.by_locus.values().sum(),
            unnamed_computed: self.computed.values().sum(),
            unnamed_python_set: self.python_bounded.values().sum(),
            unnamed_javascript: self.javascript_unnamed,
            refused_here: self.turned_away.total(),
            table_reads: self.tables.reads.values().sum(),
            table_writes: self.tables.writes.values().sum(),
            distinct_tables: self.tables.tables(),
            tables: busiest_tables(&self.tables),
            sql: rank(&self.tables.verbs),
            doing,
            renames: self.renames,
            busiest: rank_both(&self.distinct, false),
            writers: rank_both(&self.by_command, true),
            hosts: rank_both(&self.remote, false),
            unread: rank(&self.by_name),
            local_names: rank(&self.local_by_name),
            variable_names: rank(&self.from_a_variable_by_name),
            opaque_words: rank(&self.by_word),
        }
    }
}

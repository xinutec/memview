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

/// The shape of an operation, for the distribution.
///
/// Deliberately prose rather than a variant name: this string is read by
/// somebody asking what the fleet *does*, not by somebody holding the enum.
pub fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Read { .. } => "read",
        Op::Write { .. } => "write",
        Op::Remove { .. } => "remove",
        Op::Copy { .. } => "copy",
        Op::Move { .. } => "move",
        Op::Search { .. } => "search",
        Op::Transform { in_place: true, .. } => "transform (in place)",
        Op::Transform { .. } => "transform",
        Op::Run { .. } => "run a script",
        Op::Nested { .. } => "open a shell (bash -c, nix --run)",
        Op::Python { .. } => "run python (-c, or a heredoc)",
        Op::JavaScript { .. } => "run javascript (-e, or a heredoc)",
        Op::Sql { .. } => "query a database",
        Op::Remote { .. } => "reach another machine (ssh, kubectl exec)",
        Op::RemoteRun { .. } => "run a program on another machine (no shell)",
        Op::ChangeDir { .. } => "cd",
        Op::Git(GitOp::Stage { .. }) => "git stage",
        Op::Git(GitOp::Alter { .. }) => "git alter",
        Op::Git(GitOp::Inspect { .. }) => "git inspect",
        Op::Git(GitOp::Other { .. }) => "git (other)",
        Op::Nothing => "nothing with files",
        Op::Unknown { .. } => "not understood",
    }
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
    /// Subjects the text does not determine. Counted beside the commands not
    /// read at all, because they are the same kind of admission: the size of
    /// what this does not know, stated by the thing that does not know it.
    pub unnamed: usize,
    pub by_word: BTreeMap<String, usize>,
    /// Subjects a glob loop BOUNDED — still unnamed, but a subset of a pattern
    /// rather than of anything at all.
    pub by_pattern: BTreeMap<String, usize>,
    /// The same admission from the Python reader: the call that wanted a path,
    /// where the path was computed and no text of it survives.
    pub computed: BTreeMap<String, usize>,
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
        // ⚠ The total comes from `subjects_not_named`, which folds in the Python
        // and JavaScript readers' accounts too — the map below is the shell's
        // words alone, and reading a total off it is the undercount memview#824
        // was about.
        self.unnamed += found.subjects_not_named();
        for (word, n) in &found.unnamed {
            *self.by_word.entry(word.clone()).or_insert(0) += n;
        }
        for (pattern, n) in &found.bounded {
            *self.by_pattern.entry(pattern.clone()).or_insert(0) += n;
        }
        for (call, n) in &found.python.unresolved {
            *self.computed.entry(call.clone()).or_insert(0) += n;
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
        self.handled + self.unhandled
    }

    /// How much of what ran the table has an entry for.
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
    pub unnamed_computed: usize,
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
            unnamed_computed: self.computed.values().sum(),
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
            opaque_words: rank(&self.by_word),
        }
    }
}

//! What each session did, in order, and how it turned out.
//!
//! The timeline. [`crate::activity`] names one command's work; this is the record of
//! that work across the whole history, each call its own turn with the result that
//! came back.
//!
//! **Derived, never verbatim.** No command line, no prompt, no output text is kept:
//! a row is an agent, a moment, a repository, a kind of work, how many commands of
//! it, and whether it worked. A viewer that served the literal history would make
//! the corpus depend on the transcripts instead of distilling them.
//!
//! Everything that happened, not the notable part of it — Pippijn's call.
//! Reading a file is smaller work than running a build and the timeline does not say
//! so: a record that quietly dropped the small things would answer "what was this
//! session doing" with a curated version of it. Weighting belongs to whatever
//! *displays* a row, which can see how many there are.
//!
//! **Dictionaries, not strings.** Agent, repository, kind and host repeat endlessly
//! across a hundred thousand rows, so they are interned and the rows carry indices.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How a piece of work turned out.
///
/// `Rejected` is not a kind of failure — it means the command never ran. Every
/// other state here is about a process that started. A file named by a rejected call
/// was never opened, and recording it invents work out of an intention.
///
/// Reading the output to tell the two apart is the one exception to the rule that
/// this must not interpret what a command printed: the harness writes one fixed
/// sentence at the start of the content, and matching it **anchored there** is
/// reading a structural marker. Anchoring is what makes it safe — sessions that
/// merely *searched* for the phrase wrote it into their own record.
///
/// `Unknown` is a real state, not a synonym for `Ok`: an interruption is a separate
/// message, so the call it stopped never gets an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Unknown,
    Ok,
    Failed,
    Rejected,
}

impl Verdict {
    /// Whether a call that does exactly **one** thing did it.
    ///
    /// A tool call is atomic — an `Edit` either replaced the text or changed nothing at
    /// all — so its result settles the matter outright, with none of the reachability
    /// reasoning a shell script needs. Without this, failed `Edit`s and `Write`s count
    /// as changes to files they left exactly as they were.
    ///
    /// `Unknown` counts: silence means the outcome went unrecorded, not that the tool
    /// declined to act.
    pub fn completed(self) -> bool {
        matches!(self, Verdict::Ok | Verdict::Unknown)
    }

    /// Whether a file use in a command reached this way may be attributed.
    ///
    /// The join between what the *text* says had to hold and what the *result* says
    /// happened — neither alone answers it. Deliberately one-sided: `true` means
    /// certain, `false` means "cannot say", never "did not run".
    /// [`crate::shell::Reached::Always`] carries most of the corpus, and it is the case
    /// the exit status cannot spoil: `a; b; c` runs all three whatever any returns.
    ///
    /// `Failed` cannot distinguish "ran and returned non-zero" from "bash refused
    /// the text", and those are opposite facts. A runtime failure attempted its reads;
    /// a *syntax* error started nothing, because bash parses its whole input before
    /// running any of it. That is the shape [`Verdict::Rejected`] is written for, but
    /// `Rejected` means the harness declined, and bash declining arrives here as an
    /// ordinary `Failed`.
    ///
    /// Left alone deliberately, on a measurement: gate 2 puts the whole corpus to bash,
    /// and the one command in it that bash will not parse extracts no reads and no
    /// writes anyway. Detecting the case needs bash in the mining path, which is minutes
    /// a run to protect zero uses — memview#1074 has the repro. Only gate 2 can see that
    /// change, because such a tree parses, round-trips and prints as valid shell.
    pub fn admits(self, reached: crate::shell::Reached) -> bool {
        use crate::shell::Reached;
        match (self, reached) {
            // Refused before it began: nothing in it ran, whatever it said. The one verdict
            // that is a fact about the *process* rather than how the process went, which is why
            // it alone overrides the text.
            (Verdict::Rejected, _) => false,
            // Everything else started. An unconditional command in a script that started is
            // the one thing no exit status can take away.
            //
            // `Unknown` — no result line at all — is read as "started, outcome unrecorded".
            // A transcript can lack results for reasons that say nothing about the shell: it
            // was interrupted, it is still running, mining caught it mid-turn. Reading silence
            // as refusal would drop every shell file use in such a transcript at once.
            (_, Reached::Always) => true,
            // Exit 0 at the end of an `&&` chain means every link in it
            // succeeded, so every link ran. Only the final segment's chain
            // reaches the reported status — the parser has already demoted the
            // rest, so this needs no further condition.
            (Verdict::Ok, Reached::OnSuccess) => true,
            _ => false,
        }
    }
}

/// One stretch of work: one kind of activity, in one turn.
///
/// Field names are one character because there are a hundred thousand of these and
/// the artefact is read over a VPN.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    /// Index into [`Doing::agents`].
    pub a: u32,
    /// Index into [`Doing::projects`]; absent for work outside any repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<u32>,
    /// Index into [`Doing::kinds`].
    pub k: u32,
    /// Minutes since the epoch. Seconds are noise at this scale and cost a
    /// third of the field's digits.
    pub t: i64,
    /// How many commands of this kind the turn contained.
    pub n: u32,
    pub v: Verdict,
    /// Index into [`Doing::hosts`], when the work was somewhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    /// Index into [`Doing::episodes`] — which instruction this was part of.
    ///
    /// Absent for work with no prompt above it in its transcript, which is what
    /// a resumed session looks like from the outside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e: Option<u32>,
}

/// One instruction, and everything done under it.
///
/// The boundary is a user's turn, and that is the only one this reads. Every
/// other candidate — a gap in time, a change of repository, a change of kind — is
/// *inferred*, and inference can merge two instructions into one, which is the error
/// that makes a grouping lie. A recorded boundary can only over-segment, which is
/// merely less useful.
///
/// It cannot be labelled by what was asked. No prompt text reaches an artefact
/// — see this module's head — so an episode is a bracket in time plus whatever its
/// own rows say.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    /// Index into [`Doing::agents`].
    pub a: u32,
    /// The minute of its first row. Not the prompt's own minute: a turn can sit
    /// unanswered, and an episode that started before any work happened would
    /// draw a gap nothing was doing.
    pub t: i64,
    /// The minute of its last.
    pub until: i64,
    /// How many rows it holds, so a page showing three of them can still say
    /// how large the stretch was.
    pub n: u32,
}

/// The timeline, with its dictionaries.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Doing {
    #[serde(default)]
    pub generated: String,
    pub agents: Vec<String>,
    pub projects: Vec<String>,
    pub kinds: Vec<String>,
    pub hosts: Vec<String>,
    /// Oldest first.
    pub rows: Vec<Row>,
    /// The instructions those rows were carried out under, oldest first.
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

/// A dictionary being built: a name to its index, once each.
#[derive(Debug, Default)]
pub struct Names {
    index: BTreeMap<String, u32>,
    list: Vec<String>,
}

impl Names {
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(at) = self.index.get(name) {
            return *at;
        }
        let at = self.list.len() as u32;
        self.index.insert(name.to_string(), at);
        self.list.push(name.to_string());
        at
    }

    pub fn into_vec(self) -> Vec<String> {
        self.list
    }

    /// Rebuild a dictionary from a frozen one, so an index already written into a row
    /// still means the same name.
    ///
    /// Positional, and that is the whole contract. [`Names::into_vec`] emits names
    /// in index order, so re-interning them in that order reproduces every index — which
    /// is what lets a resumed fold append rows beside ones it did not build. Change
    /// either side and every `a`, `p`, `k` and `h` in the carried rows silently means a
    /// different name.
    pub fn from_vec(list: Vec<String>) -> Self {
        let index = list
            .iter()
            .enumerate()
            .map(|(at, name)| (name.clone(), at as u32))
            .collect();
        Self { index, list }
    }
}

/// One stretch of work as the miner has it, before it is interned.
pub struct Work<'a> {
    /// The tool-use id, which the result will name.
    pub call: &'a str,
    pub agent: &'a str,
    pub project: Option<&'a str>,
    pub host: Option<&'a str>,
    pub kind: &'a str,
    pub n: u32,
    pub minute: i64,
}

/// The timeline under construction, before its dictionaries are frozen.
#[derive(Debug, Default)]
pub struct Log {
    pub agents: Names,
    pub projects: Names,
    pub kinds: Names,
    pub hosts: Names,
    pub rows: Vec<Row>,
    /// Rows still waiting for the result of the call that produced them, by
    /// tool-use id. A result arrives on a later line, so the row is written
    /// first and its verdict filled in when the answer comes back.
    pending: BTreeMap<String, Vec<usize>>,
    pub episodes: Vec<Episode>,
    /// The prompt seen but not yet acted on, and the episode it becomes.
    ///
    /// Materialised by the first row under it, not by the prompt itself.
    /// A turn that asked a question and got an answer did no work this records,
    /// and an episode holding nothing would be a bracket around a blank.
    prompt: Option<String>,
    current: Option<u32>,
}

impl Log {
    /// Continue the fold a previous run froze, instead of starting from nothing.
    ///
    /// Everything here carries except `pending`. A row waiting on a result that
    /// had not arrived when the artefact was written stays [`Verdict::Unknown`] forever:
    /// its answer lands in the tail, where nothing is left to match it to. Against a
    /// real watermark that is a handful of calls across the whole corpus, and carrying
    /// it would mean remapping row indices through `finish`'s sort and putting resume
    /// state on an exported wire type.
    ///
    /// The open episode is a different size of loss and does not stay here — see
    /// [`Log::reopen`].
    pub fn resume(from: Doing) -> Self {
        Self {
            agents: Names::from_vec(from.agents),
            projects: Names::from_vec(from.projects),
            kinds: Names::from_vec(from.kinds),
            hosts: Names::from_vec(from.hosts),
            rows: from.rows,
            pending: BTreeMap::new(),
            episodes: from.episodes,
            prompt: None,
            current: None,
        }
    }

    /// A new transcript begins, so no episode is open.
    ///
    /// Without this the last instruction of one session would adopt the first
    /// rows of the next file read, which are somebody else's work entirely.
    pub fn open_transcript(&mut self) {
        self.prompt = None;
        self.current = None;
    }

    /// Re-enter a transcript mid-instruction, carrying the episode a previous read left
    /// open.
    ///
    /// This is the loss a byte offset alone cannot avoid. An episode is bracketed
    /// by a user's turn, so a cut taken mid-instruction leaves every row until the
    /// *next* prompt with no episode above it. That strands far more tail calls than the
    /// unresolved results above, and unlike those it is cheap to keep, because the state
    /// is an index and a name rather than a row position — [`crate::watermark::Resume`]
    /// carries it.
    pub fn reopen(&mut self, episode: Option<u32>, prompt: Option<String>) {
        self.prompt = prompt;
        // An episode this log does not hold cannot be continued. The index comes from
        // a watermark, and a caller may legitimately not have carried the timeline. Left
        // unchecked, the next `push` indexed an empty vector and PANICKED.
        //
        // Filtered here rather than at the call site because the log is the only thing that
        // knows what it holds. Dropping to `None` starts a fresh episode, which is what "I
        // have no record of the one you mean" should do.
        self.current = episode.filter(|at| (*at as usize) < self.episodes.len());
    }

    /// The episode still open on the transcript just read, to be carried to the
    /// run that reads its tail. Pairs with [`Log::reopen`].
    pub fn open_episode(&self) -> (Option<u32>, Option<String>) {
        (self.current, self.prompt.clone())
    }

    /// A user said something: whatever follows is one instruction's worth.
    pub fn begin_episode(&mut self, agent: &str) {
        self.prompt = Some(agent.to_string());
        self.current = None;
    }

    /// Record one turn's worth of work, unresolved until its result arrives.
    pub fn push(&mut self, work: Work<'_>) {
        // The episode this row belongs to, created the moment there is a row to
        // put in it.
        if let Some(agent) = self.prompt.take() {
            let a = self.agents.intern(&agent);
            self.current = Some(self.episodes.len() as u32);
            self.episodes.push(Episode {
                a,
                t: work.minute,
                until: work.minute,
                n: 0,
            });
        }
        if let Some(at) = self.current {
            let episode = &mut self.episodes[at as usize];
            episode.t = episode.t.min(work.minute);
            episode.until = episode.until.max(work.minute);
            episode.n += 1;
        }
        let row = Row {
            a: self.agents.intern(work.agent),
            p: work.project.map(|p| self.projects.intern(p)),
            k: self.kinds.intern(work.kind),
            t: work.minute,
            n: work.n,
            v: Verdict::Unknown,
            h: work.host.map(|h| self.hosts.intern(h)),
            e: self.current,
        };
        self.pending
            .entry(work.call.to_string())
            .or_default()
            .push(self.rows.len());
        self.rows.push(row);
    }

    /// The result of a call, applied to every row it produced.
    pub fn resolve(&mut self, call: &str, verdict: Verdict) {
        let Some(rows) = self.pending.remove(call) else {
            return;
        };
        for at in rows {
            self.rows[at].v = verdict;
        }
    }

    /// Freeze into the artefact, oldest first.
    /// As [`Log::finish`], and also the map from each episode's OLD index to its
    /// canonical one.
    ///
    /// A caller holding episode indices of its own MUST remap them. The resume
    /// watermarks record `open_episode()` during the scan, against the pre-canonical
    /// numbering; saved unremapped they would name a different instruction on the next
    /// run, silently (memview#1240).
    pub fn finish_canonical(
        mut self,
        generated: &str,
    ) -> (Doing, std::collections::BTreeMap<u32, u32>) {
        // A TOTAL order, not just the minute. `sort_by_key(|row| row.t)` is stable,
        // so rows sharing a minute kept their INSERTION order — the order transcripts
        // happened to be read in, which differs between a whole scan and a resumed one.
        // Every field the row carries in its own right takes part; `e` cannot, because it
        // is renumbered below.
        self.rows.sort_by(|x, y| {
            (x.t, x.a, x.p, x.k, x.n, x.h, x.v).cmp(&(y.t, y.a, y.p, y.k, y.n, y.h, y.v))
        });
        // Episode identity was its POSITION in this vector, assigned as
        // `episodes.len()` at creation, so it depended on when the scan reached it — which
        // is exactly what reading only the changed transcripts alters (memview#1240).
        //
        // Renumbered here in the order an episode is first REFERENCED by the sorted rows,
        // so the numbering is a property of the content rather than of the traversal. An
        // episode holds at least one row by construction, so none is dropped.
        let mut canonical: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        let mut order: Vec<u32> = Vec::new();
        for row in &self.rows {
            if let Some(old) = row.e {
                canonical.entry(old).or_insert_with(|| {
                    order.push(old);
                    (order.len() - 1) as u32
                });
            }
        }
        let episodes: Vec<Episode> = order
            .iter()
            .map(|old| self.episodes[*old as usize].clone())
            .collect();
        for row in &mut self.rows {
            if let Some(old) = row.e {
                row.e = canonical.get(&old).copied();
            }
        }
        self.episodes = episodes;
        let done = Doing {
            generated: generated.to_string(),
            agents: self.agents.into_vec(),
            projects: self.projects.into_vec(),
            kinds: self.kinds.into_vec(),
            hosts: self.hosts.into_vec(),
            rows: self.rows,
            episodes: self.episodes,
        };
        (done, canonical)
    }

    /// Freeze the timeline. Use [`Log::finish_canonical`] when episode indices
    /// are held elsewhere and have to be remapped.
    pub fn finish(self, generated: &str) -> Doing {
        self.finish_canonical(generated).0
    }
}

impl Doing {
    pub fn load(path: &std::path::Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        // Compact rather than pretty: a hundred thousand rows of indentation is
        // a third of the file and nobody reads it by eye.
        std::fs::write(path, serde_json::to_string(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// The `cd` targets the shell said it could not enter, read off a call's output.
///
/// **Why the text and not the exit code.** `cd nope; cat x` can succeed as a whole,
/// so a verdict cannot say the directory never moved. The shell says it in words,
/// naming the target it refused, and that is the only place it is said. It matters
/// because a `cd` the parser applies and the shell did not leaves every later
/// relative path resolved against a directory the command never entered.
///
/// **Anchored to the whole message, not to `cd: `.** That prefix alone matches prose
/// — the corpus has `cd: harden inspircd …`, a commit subject in a `git log`. What
/// identifies a refusal is the shell's own suffix after the target.
///
/// **Two shells, because the corpus has two.** bash puts the target before the
/// message and zsh puts it after:
///
/// ```text
/// bash: line 1: cd: memcheck: No such file or directory
/// (eval):cd:1: no such file or directory: src
/// ```
///
/// This read bash's form only, on the stated grounds that no measured call used
/// zsh's. That was wrong by a lot: zsh-worded refusals are nearly as common, so a
/// large fraction of the corpus's refusals were invisible. `SHELL` says what the
/// session's own shell is, not what the `nix develop -c`, `nix-shell --run` and
/// `ssh` invocations inside these commands run.
///
/// The zsh grammar is anchored on the message *in position*, so the same prose that
/// `cd: ` alone would match cannot reach it.
pub fn refused_dirs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut keep = |target: &str| {
        let target = target.trim().trim_end_matches('/');
        if !target.is_empty() && !out.iter().any(|seen| seen == target) {
            out.push(target.to_string());
        }
    };
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(target) = refused_by_bash(line) {
            keep(target);
        } else if let Some(target) = refused_by_zsh(line) {
            keep(target);
        }
    }
    out
}

/// Every wording the two readers below accept, as bytes to scan for.
///
/// A caller that prescans MUST use this, and nothing narrower. `agents::
/// refusals` had its own one-needle gate before parsing a line, and that silently
/// decided what [`refused_dirs`] would ever be asked about: zsh's wording never
/// reached it, and neither did bash's own `Not a directory`. Widening the parser
/// alone changed nothing measurably, because the gate in front of it was the real
/// limit. Two places holding one list is how that happened; this is the list.
pub const REFUSAL_PHRASES: [&str; 4] = [
    "No such file or directory",
    "no such file or directory",
    "Not a directory",
    "not a directory",
];

/// Whether a line could carry a refusal at all — the cheap test worth running
/// before paying to parse one.
///
/// Bytes rather than `str`, because the caller is scanning a transcript line it
/// has not decoded yet, and deciding to decode it is the whole point.
#[must_use]
pub fn may_hold_refusal(line: &[u8]) -> bool {
    REFUSAL_PHRASES
        .iter()
        .any(|phrase| line.windows(phrase.len()).any(|at| at == phrase.as_bytes()))
}

/// bash: the target sits between `cd: ` and the message at the end of the line.
///
/// The LAST `cd: ` on the line: bash prefixes its own name and the line number —
/// `bash: line 1: cd: memcheck: …` — and a path in the target could itself
/// contain the needle.
fn refused_by_bash(line: &str) -> Option<&str> {
    const ENDINGS: [&str; 2] = ["No such file or directory", "Not a directory"];
    let at = line.rfind("cd: ")?;
    let rest = &line[at + "cd: ".len()..];
    ENDINGS.iter().find_map(|ending| {
        rest.strip_suffix(ending)
            .and_then(|head| head.strip_suffix(": "))
    })
}

/// zsh: the message comes first and the target is the rest of the line.
///
/// `cd:1: no such file or directory: src`, and `(eval):cd:1: …` when the command
/// reached the shell through `eval` — which is how a `nix develop -c` or an
/// `ssh` one-liner usually arrives. The line number is optional.
fn refused_by_zsh(line: &str) -> Option<&str> {
    const ENDINGS: [&str; 2] = ["no such file or directory: ", "not a directory: "];
    let at = line.rfind("cd:")?;
    let rest = line[at + "cd:".len()..].trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.strip_prefix(':').unwrap_or(rest).strip_prefix(' ')?;
    ENDINGS
        .iter()
        .find_map(|ending| rest.strip_prefix(ending))
        .filter(|target| !target.is_empty())
}

/// Minutes since the epoch, from an ISO-8601 stamp.
///
/// Parsed by hand rather than through chrono: the stamps are all
/// `YYYY-MM-DDTHH:MM:SS…Z` from one producer, and the miner reads millions of
/// them.
pub fn minute(stamp: &str) -> Option<i64> {
    let bytes = stamp.as_bytes();
    if bytes.len() < 16 {
        return None;
    }
    let num = |from: usize, to: usize| stamp.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, min) = (num(11, 13)?, num(14, 16)?);
    Some((days_from_civil(year, month, day) * 24 + hour) * 60 + min)
}

/// Howard Hinnant's civil-days algorithm, as the viewer's roster uses it — the
/// whole need is a day number, and a date crate would be a dependency for it.
/// Twice over now that this is a leaf: see `Cargo.toml` for why the list there
/// is the boundary rather than a convenience.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

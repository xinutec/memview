//! What each session did, in order, and how it turned out.
//!
//! The timeline. [`crate::activity`] names one command's work; this is the
//! record of that work across the whole history — 90,166 Bash calls, each one
//! its own turn, each with the result that came back.
//!
//! **Derived, never verbatim.** No command line, no prompt, no output text is
//! kept: a row is an agent, a moment, a repository, a kind of work, how many
//! commands of it, and whether it worked. That is the surviving half of
//! `feedback_memview_distils_never_serves_history` — Pippijn lifted the
//! no-timeline half on 2026-08-02 and left this one standing, for the reason it
//! was written: a viewer that serves the literal history makes the corpus
//! depend on the transcripts instead of distilling them.
//!
//! **Dictionaries, not strings.** The agent, repository, kind and host of every
//! row repeat endlessly across a hundred thousand of them, so they are interned
//! and the rows carry indices. It is what keeps the artefact in the same order
//! of size as the roster beside it.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How a piece of work turned out.
///
/// ⚠ **`Rejected` is not a kind of failure — it means the command never ran.**
/// Every other state here is about a process that started; this one is about one
/// that did not exist. A file named by a rejected call was never opened, and
/// recording it invents work out of an intention.
///
/// Reading the output text to tell the two apart is the one exception to the
/// rule that this must not interpret what a command printed. It is not
/// interpretation: the harness writes one fixed sentence at the start of the
/// content, and matching it anchored there is reading a structural marker, not a
/// program's stderr. Anchoring is what makes it safe — the same sentence
/// appears 167 times across the transcripts and only 92 are real, the rest being
/// sessions like this one that merely *searched* for the phrase and wrote it
/// into their own record.
///
/// `Unknown` is a real state, not a synonym for `Ok`: an interruption is not a
/// result at all but a separate message, so the call it stopped simply never
/// gets an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Unknown,
    Ok,
    Failed,
    Rejected,
}

impl Verdict {
    /// Whether a command under this condition certainly ran.
    ///
    /// The join between what the *text* says had to hold and what the *result*
    /// says happened — neither alone answers it. Deliberately one-sided: `true`
    /// means certain, `false` means "cannot say", never "did not run". A file
    /// use may only be attributed to somebody on a `true`.
    ///
    /// [`crate::shell::Reached::Always`] carries most of the corpus, and it is
    /// the case the exit status cannot spoil: `a; b; c` runs all three whatever
    /// any of them returns.
    /// Whether a call that does exactly **one** thing did it.
    ///
    /// A tool call is atomic — an `Edit` either replaced the text or changed
    /// nothing at all — so its result settles the matter outright, with none of
    /// the reachability reasoning a shell script needs. Without this, 990 failed
    /// `Edit`s and 289 failed `Write`s count as changes to files they left
    /// exactly as they were.
    ///
    /// `Unknown` counts: silence means the outcome went unrecorded, not that the
    /// tool declined to act.
    pub fn completed(self) -> bool {
        matches!(self, Verdict::Ok | Verdict::Unknown)
    }

    pub fn admits(self, reached: crate::shell::Reached) -> bool {
        use crate::shell::Reached;
        match (self, reached) {
            // Refused before it began: nothing in it ran, whatever it said.
            // The one verdict that is a fact about the *process*, not about how
            // the process went, which is why it alone overrides the text.
            (Verdict::Rejected, _) => false,
            // Everything else started. An unconditional command in a script
            // that started is the one thing no exit status can take away.
            //
            // `Unknown` — no result line at all — is read as "started, outcome
            // unrecorded" rather than "never ran". A transcript can lack results
            // for reasons that say nothing about the shell: it was interrupted,
            // it is still running, mining caught it mid-turn. Reading silence as
            // refusal would drop every shell file use in such a transcript at
            // once, which is a far larger error than the 12 calls it protects.
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
/// Field names are one character because there are a hundred thousand of these
/// and the artefact is read over a VPN.
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
}

impl Log {
    /// Record one turn's worth of work, unresolved until its result arrives.
    pub fn push(&mut self, work: Work<'_>) {
        let row = Row {
            a: self.agents.intern(work.agent),
            p: work.project.map(|p| self.projects.intern(p)),
            k: self.kinds.intern(work.kind),
            t: work.minute,
            n: work.n,
            v: Verdict::Unknown,
            h: work.host.map(|h| self.hosts.intern(h)),
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
    pub fn finish(mut self, generated: &str) -> Doing {
        self.rows.sort_by_key(|row| row.t);
        Doing {
            generated: generated.to_string(),
            agents: self.agents.into_vec(),
            projects: self.projects.into_vec(),
            kinds: self.kinds.into_vec(),
            hosts: self.hosts.into_vec(),
            rows: self.rows,
        }
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
/// **Why the text and not the exit code.** `cd nope; cat x` can succeed as a
/// whole — the `cd` failed, `cat` ran, the call exits 0 — so a verdict cannot
/// say that the directory never moved. The shell says it in words, naming the
/// target it refused, and that is the only place it is said.
///
/// This matters because a `cd` the parser applies but the shell did not leaves
/// every later relative path in the script resolved against a directory the
/// command never entered. Measured across this project's transcripts: **247
/// results report one.**
///
/// **Anchored to the whole message, not to `cd: `.** That prefix alone matches
/// prose — the corpus has `cd: TLS handshake + banner (amun.xinutec.org)` and
/// `cd: harden inspircd …`, which are commit subjects in a `git log`, not a
/// shell. What identifies a refusal is the shell's own suffix after the target.
///
/// **Two shells, because the corpus has two.** bash puts the target before the
/// message and zsh puts it after:
///
/// ```text
/// bash: line 1: cd: memcheck: No such file or directory
/// (eval):cd:1: no such file or directory: src
/// ```
///
/// ⚠ This read bash's form only, on the stated grounds that *"no measured call
/// in the corpus uses"* zsh's. That was wrong, and it was wrong by a lot:
/// measured 2026-08-12, **74 calls carry a zsh-worded refusal and 99 a
/// bash-worded one**, so 43% of the refusals in the corpus were invisible — and
/// every one of them is a `cd` the parser applied and the shell did not, which
/// is precisely the failure this function exists to prevent. The claim had no
/// measurement behind it; `SHELL` being `bashInteractive` says what the session's
/// own shell is, not what the thousands of `nix develop -c`, `nix-shell --run`
/// and `ssh` invocations inside these commands run.
///
/// The zsh grammar is anchored on the message *in position* — `cd`, an optional
/// line number, then the message, then the target — so the same prose that
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
/// ⚠ **A caller that prescans MUST use this, and nothing narrower.** `agents::
/// refusals` had its own `const ENDING: &[u8] = b"No such file or directory"` as
/// a cheap gate before parsing a line, and that single needle silently decided
/// what [`refused_dirs`] would ever be asked about: zsh's lower-cased wording
/// never reached it, and neither did bash's own `Not a directory`, which this
/// function has always handled. Widening the parser alone changed **nothing**
/// measurably — 91 refusals to 102 — because the gate in front of it was the
/// real limit. Two places holding one list is how that happened; this is the
/// list.
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

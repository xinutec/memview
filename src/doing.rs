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
/// Two states and an absence, deliberately: the transcripts say whether the
/// call errored and nothing finer without reading the output text, which is the
/// line this must not cross. `Unknown` is rare — 12 calls in the whole corpus
/// have no result at all — but it is a real state and not a synonym for `Ok`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Unknown,
    Ok,
    Failed,
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
    pub fn resolve(&mut self, call: &str, failed: bool) {
        let Some(rows) = self.pending.remove(call) else {
            return;
        };
        let verdict = if failed { Verdict::Failed } else { Verdict::Ok };
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

/// Howard Hinnant's civil-days algorithm, as [`crate::agents`] uses it — the
/// whole need is a day number, and a date crate would be a dependency for it.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

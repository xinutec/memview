//! What each turn actually did to which file, with the command that did it.
//!
//! [`crate::doing`] answers *what was this agent working on, and when* — one row
//! per kind of work in a turn, which is the right grain for a timeline. This
//! answers the question a reader asks next, standing on one of those rows: **which
//! files, and how do you know?** One row per file use, each carrying the command
//! it came from.
//!
//! ⚠ **A derived artefact keyed to the timeline, not a mirror of the history.**
//! `scripts/sync.sh` removed a mined `history.json` for two reasons, and only one
//! of them was privacy — Pippijn settled that on 2026-08-13 (*"Isis should be
//! trusted. Everything can go there."*), so the command text travels in full. The
//! other reason stands on its own: memview is for reading the memory documents
//! well, and a viewer that also served the literal history made the corpus depend
//! on the transcripts instead of distilling them. So a command travels **because
//! a claim needs the command it rests on**, never so the history can be browsed.
//! Nothing here is keyed by session or ordered as a conversation.
//!
//! ⚠ **`doing.json` does not change.** A row here carries `(agent, minute)`
//! itself, which is the key a timeline row already has, so opening a turn is a
//! filter rather than a join and no published format is touched.
//!
//! # Why the dictionaries, and why the command is in one
//!
//! Measured over 120,427 Bash calls: the text of every distinct *whole call* is
//! 41.5 MB, which is most of an artefact by itself. But an effect does not rest
//! on a whole call — it rests on the one simple command inside it that touched
//! the file, and **most simple commands touch nothing**. A `cd`, an `echo`, a
//! bare `head` at the end of a pipe never appears in a row, so its text is never
//! stored. The dictionary of commands that do bear an effect is **9.7 MB over
//! 127,028 entries** (memview#93).
//!
//! That is a saving from *relevance*, not from deduplication: simple commands
//! dedupe worse than whole calls do — 275,892 distinct against 115,578 — because
//! unrolling a loop multiplies them.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::doing::{Names, Verdict};
use crate::shell::Reached;

/// What a row says happened to a file.
///
/// Narrower than [`crate::activity::Activity`], which describes what a *turn* was
/// doing. Here the question is only what became of one path, because that is what
/// a reader is checking when they open a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Did {
    /// Opened and read.
    #[serde(rename = "r")]
    Read,
    /// Changed: a `>` redirect, `sed -i`, the destination of a `cp`, an `rm`.
    #[serde(rename = "w")]
    Wrote,
    /// Searched *in* — the path was consulted, and what was looked for is the
    /// point. Kept apart from a plain read because "who grepped for this" and
    /// "who read this" are different questions.
    #[serde(rename = "s")]
    Searched,
    /// Named a subject the text does not determine. The path field holds the
    /// **pattern** it is a subset of, or is absent when nothing bounds it — see
    /// [`crate::shell_files::Extract::bounded`].
    #[serde(rename = "u")]
    Unnamed,
}

/// One thing a turn did to one file.
///
/// Field names are one character for the reason [`crate::doing::Row`] gives:
/// there are hundreds of thousands of these and the artefact is read over a VPN.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    /// Index into [`Effects::agents`].
    pub a: u32,
    /// Minutes since the epoch — the same clock as [`crate::doing::Row::t`], and
    /// the half of the key that makes this a filter rather than a join.
    pub t: i64,
    /// What became of it.
    pub k: Did,
    /// Index into [`Effects::paths`]; absent when the subject was not named and
    /// nothing bounded it either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<u32>,
    /// Index into [`Effects::patterns`], for what a search was looking for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<u32>,
    /// Index into [`Effects::hosts`], when the file is on another machine.
    ///
    /// ⚠ **Never mixed into the local paths.** The path exists there and not
    /// here; a row without this field is a claim about this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    /// Index into [`Effects::commands`] — the command that did it.
    pub c: u32,
    /// What had to hold for that command to run.
    pub r: Reached,
    /// What the call returned, so a reader can tell "this happened" from "this
    /// was in the script".
    pub v: Verdict,
}

/// The effects, with their dictionaries.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Effects {
    #[serde(default)]
    pub generated: String,
    pub agents: Vec<String>,
    pub paths: Vec<String>,
    pub patterns: Vec<String>,
    pub commands: Vec<String>,
    pub hosts: Vec<String>,
    /// Oldest first.
    pub rows: Vec<Row>,
}

/// One effect as the miner has it, before it is interned.
pub struct Effect<'a> {
    /// The tool-use id, which the result will name.
    pub call: &'a str,
    pub agent: &'a str,
    pub minute: i64,
    pub did: Did,
    /// The file, or the pattern an unnamed subject is bounded by.
    pub path: Option<&'a str>,
    pub pattern: Option<&'a str>,
    pub host: Option<&'a str>,
    /// The command that did it, as the shell would have run it.
    pub command: &'a str,
    pub reached: Reached,
}

/// The effects under construction, before the dictionaries are frozen.
#[derive(Debug, Default)]
pub struct Log {
    agents: Names,
    paths: Names,
    patterns: Names,
    commands: Names,
    hosts: Names,
    rows: Vec<Row>,
    /// Rows waiting for the result of the call that produced them. The result
    /// arrives on a later transcript line, so the row is written first and its
    /// verdict filled in when the answer comes back — as in [`crate::doing::Log`].
    pending: BTreeMap<String, Vec<usize>>,
}

impl Log {
    /// Continue the fold a previous run froze, instead of starting from nothing.
    ///
    /// ⚠ **`pending` does not carry, and that is measured rather than assumed** —
    /// see [`crate::doing::Log::resume`], which loses the same three calls a
    /// night for the same reason. There is no episode state here, so this is the
    /// whole of it: the dictionaries rebuild positionally and the rows keep the
    /// indices they were written with.
    pub fn resume(from: Effects) -> Self {
        Self {
            agents: Names::from_vec(from.agents),
            paths: Names::from_vec(from.paths),
            patterns: Names::from_vec(from.patterns),
            commands: Names::from_vec(from.commands),
            hosts: Names::from_vec(from.hosts),
            rows: from.rows,
            pending: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, effect: Effect<'_>) {
        let row = Row {
            a: self.agents.intern(effect.agent),
            t: effect.minute,
            k: effect.did,
            p: effect.path.map(|path| self.paths.intern(path)),
            q: effect.pattern.map(|q| self.patterns.intern(q)),
            h: effect.host.map(|host| self.hosts.intern(host)),
            c: self.commands.intern(effect.command),
            r: effect.reached,
            v: Verdict::Unknown,
        };
        self.pending
            .entry(effect.call.to_string())
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

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Freeze into the artefact, oldest first.
    pub fn finish(mut self, generated: &str) -> Effects {
        // ⚠ **A TOTAL order, for the reason `doing::Log::finish` has one.**
        // `sort_by_key(|row| row.t)` is stable, so rows sharing a minute kept
        // their INSERTION order — the order transcripts happened to be read in,
        // which differs between a whole scan and a resumed one. This artefact
        // was the last of the four still failing the corpus parity check on
        // 2026-08-30, at identical byte length and a different hash
        // (memview#1240).
        self.rows.sort_by(|x, y| {
            (x.t, x.a, x.p, x.q, x.h, x.c, x.k, x.r, x.v)
                .cmp(&(y.t, y.a, y.p, y.q, y.h, y.c, y.k, y.r, y.v))
        });
        Effects {
            generated: generated.to_string(),
            agents: self.agents.into_vec(),
            paths: self.paths.into_vec(),
            patterns: self.patterns.into_vec(),
            commands: self.commands.into_vec(),
            hosts: self.hosts.into_vec(),
            rows: self.rows,
        }
    }
}

impl Effects {
    pub fn load(path: &std::path::Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        // Compact rather than pretty, for the reason `doing.json` is: the
        // indentation would be a third of the file and nobody reads it by eye.
        std::fs::write(path, serde_json::to_string(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

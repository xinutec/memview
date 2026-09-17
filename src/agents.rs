//! Which named Claude session works on which part of the codebase: what each
//! one actually opened and changed, counted per project directory.
//!
//! The signal is the file paths of tool calls, not `cwd` and not any text —
//! MEMORY.md names every project and is injected everywhere. Reads and writes
//! are counted apart: consulting a repository and being responsible for it are
//! different claims. Where an agent works is decided by recent days present
//! ([`recency`]), since a session is renamed as its job changes.
//!
//! Only names, project names and integers leave this module ([`crate::couse`]).
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::couse::{field, find_at, last_at};

/// One named session, and where its work actually landed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Agent {
    /// The name it goes by — "recall", "health" — or its session id when it was
    /// never named.
    pub name: String,
    /// Main-loop transcripts filed under this name. More than one when a
    /// session is resumed, or when the same name has been reused over time.
    pub transcripts: usize,
    /// Transcripts of subagents and workflow agents this session dispatched.
    /// Their work is counted as this agent's — see [`transcripts_under`].
    #[serde(default)]
    pub delegated: usize,
    /// The session ids filed under this name — the join between a memory's
    /// `originSessionId` and the roster.
    #[serde(default)]
    pub sessions: BTreeSet<String>,
    /// Files opened, per project directory. Lifetime totals, undecayed.
    pub reads: BTreeMap<String, usize>,
    /// Files written or edited, per project directory. Lifetime, undecayed.
    pub writes: BTreeMap<String, usize>,
    /// Every file this agent touched under the code root, keyed by its path
    /// relative to that root. [`reads`](Self::reads) keep only the first segment,
    /// which refuses everything finer than "which repository". About 7,300 distinct
    /// paths across the whole history, so no cap. Build output is left out — see
    /// [`attributable`].
    #[serde(default)]
    pub paths: BTreeMap<String, MemoryUse>,
    /// The same, for files used by shell commands — `sed -i`, `cp`, a `>` redirect —
    /// unioned with [`paths`](Self::paths) by [`Agents::who_works_on`] at query time.
    /// A separate map so the existing figures keep their meaning: two thirds of the
    /// fleet's shell commands touch files, and the `Write`/`Edit` miner sees none of
    /// it. "Shell" is the Bash call, so a `python3 -` heredoc lands here too.
    #[serde(default)]
    pub shell_paths: BTreeMap<String, MemoryUse>,
    /// Lines committed, per repo-relative path — the only dimension that measures
    /// SIZE rather than counting operations, and the only evidence that survived
    /// review. Attributed by [`crate::commits`]'s earliest-mention rule; a commit no
    /// transcript mentions is reported as unattributed.
    #[serde(default)]
    pub commit_lines: BTreeMap<String, LineDelta>,
    /// Files used on OTHER machines, keyed `host:/absolute/path`. Entirely from
    /// `ssh`/`kubectl exec` payloads; git cannot attribute them. Kept apart from
    /// [`paths`](Self::paths): `/etc/nixos` exists on odin and not here.
    #[serde(default)]
    pub remote_paths: BTreeMap<String, MemoryUse>,
    /// Commits attributed to this agent, across every repository.
    #[serde(default)]
    pub commits: usize,
    /// Which memories this agent works with, keyed by memory name — what it has
    /// CONSULTED, where `reads`/`writes` say where it is responsible.
    #[serde(default)]
    pub memories: BTreeMap<String, MemoryUse>,
    /// Recency-weighted days present, per project — the ordering signal. See
    /// [`recency`] for why this is days rather than files.
    #[serde(default)]
    pub recent_reads: BTreeMap<String, f64>,
    #[serde(default)]
    pub recent_writes: BTreeMap<String, f64>,
    /// First and last activity, ISO-8601.
    pub first: String,
    pub last: String,
}

/// How one agent uses one memory: the times it deliberately opened or changed
/// the file. Counted from the tool call's `file_path`, not from the memory being
/// named: a single injected sentence naming one memory recurred 3,370 times in
/// one transcript.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MemoryUse {
    /// Times this agent opened the memory with `Read`.
    pub reads: usize,
    /// Times this agent wrote or edited it — the strongest claim to it.
    pub edits: usize,
    /// Times a command that MAY have opened it did — a weaker claim, kept apart: a
    /// command after `&&` runs only if what came before it worked, and one exit
    /// status for a whole script often cannot say (19,256 such uses in the corpus).
    /// Always zero for tool calls, which are atomic.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub maybe_reads: usize,
    /// Times a corpus-wide search PRINTED A LINE of this memory back — a third kind
    /// of evidence. `reads` overstates it, silence understates it, and it is not
    /// `maybe_reads` either (memview#1238). `reads` must keep its meaning: the #884
    /// study is recomputed from the transcripts on every mine. Never added into
    /// BREADTH without a decision: 8 agents run corpus-wide greps.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub grep_matches: usize,
    /// Times a command that **may** have changed it did. See
    /// [`maybe_reads`](Self::maybe_reads).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub maybe_edits: usize,
}

/// Kept out of the artefact when nothing is uncertain, which is most entries;
/// the file is read over a VPN.
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What one agent's commits did to one file. Added and deleted stay apart: a
/// rewrite that removes 181 lines and adds 594 is not writing 413 from nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LineDelta {
    pub added: usize,
    pub deleted: usize,
    /// Commits touching this path — so a thousand lines in one sitting reads
    /// differently from a thousand across twenty.
    pub commits: usize,
}

/// One agent's answer to "who works on this", with the evidence attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkMatch {
    pub name: String,
    /// Writes and edits across every matching file — the ranking signal.
    pub edits: usize,
    /// Reads across the same files. Reported, never added to `edits`: consulting
    /// a subtree and being responsible for it are different claims.
    pub reads: usize,
    /// Lines committed across the matching files, and the commits that carried
    /// them. Reported beside the counts, never folded in: this is the same work
    /// measured a second way, and adding the two would count it twice.
    #[serde(default)]
    pub added: usize,
    #[serde(default)]
    pub deleted: usize,
    /// File changes committed, not commits: one commit touching four matching files
    /// counts four, since the per-path record cannot recover a distinct count.
    #[serde(default)]
    pub file_commits: usize,
    /// Machines this row's evidence touches, other than this one. Empty for
    /// work done entirely here — so a reader can see at a glance that a total
    /// includes another host before reading the files.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// The matching files, heaviest first — the evidence for the row above.
    pub files: Vec<WorkFile>,
}

/// One file a query matched, and how one agent used it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkFile {
    pub path: String,
    /// The names this file used to have, newest last — why a file created last
    /// week can carry a year of history.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub was: Vec<String>,
    /// Every use, tool call and shell command together.
    pub reads: usize,
    pub edits: usize,
    /// How much of the above came from a `Bash` call rather than `Write` and `Edit`,
    /// so a file with forty changes and no tool edits reads as `sed` work, not a bug.
    pub shell_reads: usize,
    pub shell_edits: usize,
    /// The machine this file is on, when it is not this one. `None` is local.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Lines this agent committed to the file, and in how many commits.
    #[serde(default)]
    pub added: usize,
    #[serde(default)]
    pub deleted: usize,
    #[serde(default)]
    pub commits: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Agents {
    /// When this was mined, ISO-8601: an mtime records the last copy, not the last
    /// derivation.
    #[serde(default)]
    pub generated: String,
    /// Commits found under the code root, and how many no transcript mentions.
    /// Reported: plenty of this history predates the corpus.
    #[serde(default)]
    pub commits: usize,
    #[serde(default)]
    pub unattributed: usize,
    /// The timeline, mined in the same pass and written to its own file. Never
    /// serialised with the roster: a hundred times the size.
    #[serde(skip)]
    pub doing: reader::doing::Doing,
    /// What each turn did to which file, with the command that did it. Never
    /// serialised with the roster, like [`Self::doing`].
    #[serde(skip)]
    pub effects: reader::effects::Effects,
    /// Where each renamed file ended up, old name to current — kept so a query for
    /// a file's old name still finds it.
    #[serde(default)]
    pub renames: BTreeMap<String, String>,
    /// When each memory was opened and changed, corpus-wide — the evidence for
    /// which of them the index should still carry. Never serialised with the roster;
    /// the miner saves it beside, where `memory-rank` reads it. See [`MemoryDays`].
    #[serde(skip)]
    pub memory_days: BTreeMap<String, MemoryDays>,
    /// Named sessions, busiest first.
    pub agents: Vec<Agent>,
}

/// The index's stem, which lives in the corpus directory and is not a memory.
pub const INDEX_STEM: &str = "MEMORY";

/// What a mined artefact has NOT seen. A `generated` field nobody consults is
/// not a safeguard — three wrong analyses came from one (#1210); the stamp has
/// to be a refusal at the point a number is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Freshness {
    /// The artefact's own stamp.
    pub generated: String,
    /// Inputs modified after it, newest first. Empty means the artefact has
    /// seen everything.
    pub unseen: Vec<String>,
}

impl Freshness {
    pub fn is_stale(&self) -> bool {
        !self.unseen.is_empty()
    }
}

/// Which memories were written after `generated`, from the history itself.
///
/// The transcripts are the record, not the filesystem: an mtime records a touch,
/// and a sweep that rewrites files without changing a word would alarm on every
/// one. The derived artefacts cannot answer either — `memory-days.json` is mined
/// by the same pass. Tail first, whole file only if the tail says so. The running
/// session is excluded, or this refuses ALWAYS.
pub fn freshness(
    generated: &str,
    roots: &[&Path],
    live_session: Option<&str>,
    home: &str,
) -> Freshness {
    let mut unseen: Vec<(String, String)> = Vec::new();
    for root in roots {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    stack.push(path);
                    continue;
                }
                // The live session writes as this runs; see above.
                if let Some(live) = live_session
                    && path.to_string_lossy().contains(live)
                {
                    continue;
                }
                if !reader::transcript::is_transcript(&path) {
                    continue;
                }
                // One tail read decides whether the whole file is worth opening.
                let Some(newest) = newest_event(&path) else {
                    continue;
                };
                if newest.as_str() <= generated {
                    continue;
                }
                // An edit to the index is not staleness: both readers load `MEMORY.md` live,
                // and a refusal that fires on a harmless change is one people learn to override.
                let a_memory = |name: &String| name != INDEX_STEM;
                for name in memories_written_after(&path, generated)
                    .into_iter()
                    .filter(a_memory)
                {
                    unseen.push((newest.clone(), name));
                }
                for name in shell_written_after(&path, generated, home)
                    .into_iter()
                    .filter(a_memory)
                {
                    unseen.push((newest.clone(), name));
                }
            }
        }
    }
    unseen.sort_by(|a, b| b.0.cmp(&a.0));
    unseen.dedup_by(|a, b| a.1 == b.1);
    Freshness {
        generated: generated.to_string(),
        unseen: unseen.into_iter().map(|(_, path)| path).collect(),
    }
}

/// The newest event timestamp in a transcript, from its tail: append-mostly, so
/// the last complete line carries the latest event even when earlier stretches
/// were rewritten.
fn newest_event(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: i64 = 64 * 1024;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL as u64);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut tail = String::new();
    file.take(TAIL as u64 * 2).read_to_string(&mut tail).ok()?;
    // Last wins: the newest event is the last one written.
    tail.rmatch_indices("\"timestamp\":\"")
        .find_map(|(at, marker)| {
            let rest = &tail[at + marker.len()..];
            rest.find('"').map(|end| rest[..end].to_string())
        })
}

/// The memories a transcript records being written after `generated`. Names,
/// not paths.
fn memories_written_after(path: &Path, generated: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        // Cheap reject first: most lines mention no memory at all.
        if !line.contains("/memory/") {
            continue;
        }
        let Some(at) = line.find("\"timestamp\":\"") else {
            continue;
        };
        let rest = &line[at + 13..];
        let Some(end) = rest.find('"') else { continue };
        if &rest[..end] <= generated {
            continue;
        }
        // A write names its file; a read cannot change what a mine should have seen.
        if !line.contains("\"Write\"") && !line.contains("\"Edit\"") {
            continue;
        }
        // Anchored on the argument, never a bare `/memory/`: prose such as
        // "memory/preferences cannot fulfil them" invented a memory.
        for (start, marker) in line.match_indices("\"file_path\":\"") {
            let rest = &line[start + marker.len()..];
            let Some(end) = rest.find('"') else { continue };
            let path = &rest[..end];
            let Some(name) = path
                .rsplit_once("/memory/")
                .map(|(_, leaf)| leaf)
                .and_then(|leaf| leaf.strip_suffix(".md"))
            else {
                continue;
            };
            if !name.contains('/') && !found.contains(&name.to_string()) {
                found.push(name.to_string());
            }
        }
    }
    found
}

/// Memories a transcript records being written by a SHELL command — a heredoc
/// write is invisible to a tool-name check, and memories have been written that
/// way. Read by `reader::shell_files`, not by looking for a `>`: a substring
/// counted `echo x > /tmp/note.md`, missed `tee`, and could not tell a read
/// redirect from a write (#1218).
fn shell_written_after(path: &Path, generated: &str, home: &str) -> Vec<String> {
    // Streamed, not slurped: `read_to_string` on every transcript whose tail
    // postdates the mine cost 30s.
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for line in std::io::BufRead::lines(std::io::BufReader::new(file)).map_while(Result::ok) {
        // Cheap rejects before any parsing: most lines are neither.
        if !line.contains("\"name\":\"Bash\"") || !line.contains(".md") {
            continue;
        }
        let Some(at) = line.find("\"timestamp\":\"") else {
            continue;
        };
        let rest = &line[at + 13..];
        let Some(end) = rest.find('"') else { continue };
        if &rest[..end] <= generated {
            continue;
        }
        let Ok(row) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        for command in bash_commands(&row) {
            let Ok(parsed) = reader::project::read(&command) else {
                continue;
            };
            let cwd = row["cwd"].as_str();
            for use_ in reader::shell_files::extract(&parsed, cwd, home).files {
                // A read of a memory cannot change what a mine should have seen.
                if !use_.write {
                    continue;
                }
                if let Some(name) = memory_stem(&use_.path)
                    && !found.contains(&name)
                {
                    found.push(name);
                }
            }
        }
    }
    found
}

/// Every `Bash` command a transcript row invokes.
fn bash_commands(row: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![row];
    while let Some(node) = stack.pop() {
        match node {
            serde_json::Value::Array(items) => stack.extend(items),
            serde_json::Value::Object(map) => {
                if map.get("name").and_then(|n| n.as_str()) == Some("Bash")
                    && let Some(command) = map
                        .get("input")
                        .and_then(|i| i.get("command"))
                        .and_then(|c| c.as_str())
                {
                    out.push(command.to_string());
                }
                stack.extend(map.values());
            }
            _ => {}
        }
    }
    out
}

/// The memory a path names, or `None`. Anchored on `/memory/` in a resolved
/// path, not on a bare filename.
fn memory_stem(path: &str) -> Option<String> {
    let (_, leaf) = path.rsplit_once("/memory/")?;
    let name = leaf.strip_suffix(".md")?;
    (!name.is_empty() && !name.contains('/')).then(|| name.to_string())
}

impl Agents {
    /// What this mine has not seen. See [`freshness`].
    pub fn freshness(&self, roots: &[&Path], live_session: Option<&str>, home: &str) -> Freshness {
        freshness(&self.generated, roots, live_session, home)
    }

    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::atomic::write(path, serde_json::to_string_pretty(self)?.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// The agent a session id belongs to, for resolving a memory's `originSessionId`.
    /// `None` is ordinary: a memory can outlive the transcript that wrote it — not
    /// because Claude Code prunes them (memview#1240, #1247), but because a handful
    /// predate the archive.
    pub fn who_works_on(&self, query: &str) -> Vec<WorkMatch> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        // A file keeps its evidence under the names it used to have, so a query for one
        // of those must find it.
        let mut aliases: BTreeMap<&String, Vec<&String>> = BTreeMap::new();
        for (was, now) in &self.renames {
            aliases.entry(now).or_default().push(was);
        }
        let named = |path: &String| -> bool {
            path.to_lowercase().contains(&needle)
                || aliases
                    .get(path)
                    .is_some_and(|was| was.iter().any(|old| old.to_lowercase().contains(&needle)))
        };
        let mut out: Vec<WorkMatch> = self
            .agents
            .iter()
            .filter_map(|agent| {
                // The two dimensions are unioned here, so a file used both ways is one row.
                let mut merged: BTreeMap<&String, WorkFile> = BTreeMap::new();
                let matching = |(path, _): &(&String, &MemoryUse)| -> bool { named(path) };
                let blank = |path: &String| WorkFile {
                    path: path.clone(),
                    was: aliases
                        .get(path)
                        .map(|was| was.iter().map(|s| (*s).clone()).collect())
                        .unwrap_or_default(),
                    host: None,
                    reads: 0,
                    edits: 0,
                    shell_reads: 0,
                    shell_edits: 0,
                    added: 0,
                    deleted: 0,
                    commits: 0,
                };
                for (path, use_) in agent.paths.iter().filter(matching) {
                    let file = merged.entry(path).or_insert_with(|| blank(path));
                    file.reads += use_.reads;
                    file.edits += use_.edits;
                }
                for (path, use_) in agent.shell_paths.iter().filter(matching) {
                    let file = merged.entry(path).or_insert_with(|| blank(path));
                    file.reads += use_.reads;
                    file.edits += use_.edits;
                    file.shell_reads = use_.reads;
                    file.shell_edits = use_.edits;
                }
                // Committed lines are the same work measured a second way: attached to the
                // row, never added to its counts.
                for (path, delta) in agent.commit_lines.iter().filter(|(path, _)| named(path)) {
                    let file = merged.entry(path).or_insert_with(|| blank(path));
                    file.added = delta.added;
                    file.deleted = delta.deleted;
                    file.commits = delta.commits;
                }
                let mut files: Vec<WorkFile> = merged.into_values().collect();
                // Work on other machines, each row saying which: a remote path is a different path.
                let mut hosts: BTreeSet<String> = BTreeSet::new();
                for (key, use_) in agent
                    .remote_paths
                    .iter()
                    .filter(|(key, _)| key.to_lowercase().contains(&needle))
                {
                    let (host, path) = match key.split_once(':') {
                        Some((host, path)) => (host.to_string(), path.to_string()),
                        None => (String::new(), key.clone()),
                    };
                    hosts.insert(host.clone());
                    files.push(WorkFile {
                        path,
                        // Renames are git's knowledge, and git is not watching
                        // the other machine.
                        was: Vec::new(),
                        host: Some(host),
                        reads: use_.reads,
                        edits: use_.edits,
                        // Remote use can only have come from an `ssh` payload,
                        // so it is shell-derived by construction.
                        shell_reads: use_.reads,
                        shell_edits: use_.edits,
                        added: 0,
                        deleted: 0,
                        commits: 0,
                    });
                }
                if files.is_empty() {
                    return None;
                }
                files.sort_by_key(|f| {
                    (
                        std::cmp::Reverse(f.edits),
                        std::cmp::Reverse(f.reads),
                        f.path.clone(),
                    )
                });
                Some(WorkMatch {
                    name: agent.name.clone(),
                    hosts: hosts.into_iter().collect(),
                    edits: files.iter().map(|f| f.edits).sum(),
                    reads: files.iter().map(|f| f.reads).sum(),
                    added: files.iter().map(|f| f.added).sum(),
                    deleted: files.iter().map(|f| f.deleted).sum(),
                    file_commits: files.iter().map(|f| f.commits).sum(),
                    files,
                })
            })
            .collect();
        out.sort_by_key(|m| {
            (
                std::cmp::Reverse(m.edits),
                std::cmp::Reverse(m.reads),
                m.name.clone(),
            )
        });
        out
    }

    pub fn name_of_session(&self, session: &str) -> Option<&str> {
        self.agents
            .iter()
            .find(|a| a.sessions.contains(session))
            .map(|a| a.name.as_str())
    }
}

/// The tool calls worth finding in a transcript, and what each one is:
/// `Some(false)` reads a file, `Some(true)` changes one, `None` touches no path.
/// All three produce a timeline row. Taken from the corpus: `Task`, `MultiEdit`
/// and `NotebookEdit` appear not once; delegation is `Agent`.
const TOOLS: [(&str, Option<bool>); 7] = [
    ("Read", Some(false)),
    ("Write", Some(true)),
    ("Edit", Some(true)),
    ("Grep", None),
    ("Agent", None),
    ("WebFetch", None),
    ("WebSearch", None),
];

/// How long it takes for a day's presence to count half as much. Fourteen days:
/// day-presence put more agents on their own project than event decay did, and
/// the answer did not move when the half-life was halved.
pub const HALF_LIFE_DAYS: f64 = 14.0;

/// Days since the epoch for an ISO-8601 stamp. Hinnant's civil-days algorithm,
/// inline, so the miner has no date dependency.
pub fn day_number(stamp: &str) -> Option<i64> {
    let bytes = stamp.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let y: i64 = stamp.get(0..4)?.parse().ok()?;
    let m: i64 = stamp.get(5..7)?.parse().ok()?;
    let d: i64 = stamp.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// Weight a set of active days against `today`, newest counting most. Days
/// present, not files touched: one afternoon of seventy-five edits outvoted a
/// fortnight of steady work. Nothing decays to zero.
pub fn recency(days: &std::collections::BTreeSet<i64>, today: i64) -> f64 {
    weighted(days.iter().copied(), today, HALF_LIFE_DAYS)
}

/// The same, over any days and any half-life — a parameter so that the constant
/// can be doubted on demand.
pub fn weighted(days: impl Iterator<Item = i64>, today: i64, half_life: f64) -> f64 {
    let total: f64 = days
        .map(|d| 0.5f64.powf(((today - d).max(0)) as f64 / half_life))
        .sum();
    // `+ 0.0` is not redundant: Rust sums `f64` from `-0.0`, so a memory with no
    // days prints as `-0.00`.
    total + 0.0
}

/// The days one memory was opened and the days it was changed, corpus-wide. The
/// days themselves, not a score: the half-life is trustworthy only while halving
/// it does not move the answer, and a stored weight cannot be re-read. Sorted
/// and unique.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryDays {
    /// Days something opened it with `Read`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<i64>,
    /// Days something wrote or edited it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<i64>,
}

/// Fold an earlier `memory-days.json` into this run's, and report how many days
/// only the earlier record still had. A day is a historical fact, so union is the
/// merge: a fresh derivation silently dropped every day a vanished transcript had
/// contributed. A missing file is the first run; a file that will not parse is an
/// ERROR, or this run's smaller view overwrites the record.
pub fn carry_forward(
    path: &std::path::Path,
    days: &mut BTreeMap<String, MemoryDays>,
) -> Result<usize> {
    // No file at all is the first run, and is fine.
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(0);
    };
    // Fatal, not zero: 0 is indistinguishable from "nothing to carry", and the
    // first version of this had exactly that bug
    // (`feedback_a_precondition_that_can_pass_wrongly`).
    let earlier: BTreeMap<String, MemoryDays> = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} exists but will not parse — refusing to overwrite it, because that \
             would delete every day it records",
            path.display()
        )
    })?;
    let mut carried = 0usize;
    for (name, was) in earlier {
        let now = days.entry(name).or_default();
        for (mine, theirs) in [(&mut now.reads, was.reads), (&mut now.edits, was.edits)] {
            let before = mine.len();
            mine.extend(theirs);
            mine.sort_unstable();
            mine.dedup();
            carried += mine.len() - before;
        }
    }
    Ok(carried)
}

/// The days an agent was present in each project, kept apart from the counts: a
/// day is not a tally. Serialised because a resumed mine cannot rebuild it — only
/// the corpus-wide UNION is written to `memory-days.json`. See [`crate::mine::Carried`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DaysSeen {
    reads: BTreeMap<String, std::collections::BTreeSet<i64>>,
    writes: BTreeMap<String, std::collections::BTreeSet<i64>>,
    memory_reads: BTreeMap<String, std::collections::BTreeSet<i64>>,
    memory_edits: BTreeMap<String, std::collections::BTreeSet<i64>>,
}

/// The project a path belongs to: the first element under the code root. `None`
/// elsewhere, dropping the scratchpad under `/private/tmp` and the corpus
/// itself, which is counted by [`memory_of`].
fn project_of(path: &str, code_root: &str) -> Option<String> {
    let root = code_root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    let name = rest.split('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// A path's position under the code root, prefix kept, so a query naming a
/// repository works like any other substring.
fn relative_to(path: &str, code_root: &str) -> Option<String> {
    let root = code_root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    (!rest.is_empty() && rest.contains('/')).then(|| rest.to_string())
}

/// Whether a path is one work can be attributed to. Build output, dependency
/// trees and logs are touched BECAUSE of the work, not as it. Measured: 0.1% of
/// tool-call use, 4.3% of shell use; one rule for both.
fn attributable(rel: &str) -> bool {
    const GENERATED: [&str; 14] = [
        "node_modules",
        "dist",
        "build",
        "target",
        "coverage",
        "logs",
        "log",
        ".venv",
        "venv",
        "__pycache__",
        ".gradle",
        ".angular",
        ".next",
        ".cache",
    ];
    const LEFTOVER: [&str; 5] = [".log", ".bak", ".tmp", ".orig", ".rej"];
    let mut segments = rel.split('/');
    !segments.any(|s| GENERATED.contains(&s)) && !LEFTOVER.iter().any(|s| rel.ends_with(s))
}

/// Whether a path on another machine is one work can be attributed to — the
/// same idea and a different list, since a remote path answers to no code root.
fn remotely_attributable(path: &str) -> bool {
    const SCRATCH: [&str; 5] = ["/tmp/", "/var/tmp/", "/proc/", "/sys/", "/dev/"];
    !SCRATCH.iter().any(|dir| path.starts_with(dir)) && attributable(path)
}

/// The memory a path names, for paths inside the corpus directory: the filename
/// stem, `MEMORY.md` excluded. A GLOB NAMES NO MEMORY: `memory/*.md` collapsed to
/// a stem of `*` with 459 uses. Dropped rather than expanded — `grep -l x
/// memory/*.md` reads all 531 and would score them equally.
fn memory_of(path: &str, memory_root: &str) -> Option<String> {
    let root = memory_root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    let stem = rest.strip_suffix(".md")?;
    let pattern = stem.contains(['*', '?', '[', ']', '{', '}']);
    (!stem.is_empty() && !stem.contains('/') && stem != "MEMORY" && !pattern)
        .then(|| stem.to_string())
}

/// One transcript file and the session whose work it records.
struct Transcript {
    path: std::path::PathBuf,
    /// The session id that owns this work — for a delegated transcript, the session
    /// that dispatched it.
    owner: String,
    delegated: bool,
}

/// Every transcript under a project directory, attributed to its owner:
/// `<project>/<session>.jsonl`, and `<project>/<session>/subagents/…` for what
/// it dispatched. Delegated work belongs to the session that dispatched it —
/// about a tenth of all Read/Write/Edit calls, unevenly across sessions.
fn transcripts_under(projects_root: &Path) -> Vec<Transcript> {
    fn descend(dir: &Path, owner: &str, out: &mut Vec<Transcript>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `file_type` does not follow symlinks, where `is_dir` would recurse a link
            // back to an ancestor until the stack gives out.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                descend(&path, owner, out);
            } else if reader::transcript::is_transcript(&path) {
                out.push(Transcript {
                    path,
                    owner: owner.to_string(),
                    delegated: true,
                });
            }
        }
    }

    let mut out = Vec::new();
    let Ok(roots) = std::fs::read_dir(projects_root) else {
        return out;
    };
    for root in roots.flatten() {
        if !root.path().is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(root.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                // A session's own directory: everything beneath it is work it
                // dispatched, however deeply nested.
                descend(&path, &stem, &mut out);
            } else if reader::transcript::is_transcript(&path) {
                out.push(Transcript {
                    path,
                    owner: stem,
                    delegated: false,
                });
            }
        }
    }
    // A session's own transcript before anything it dispatched, so the name is
    // resolved before a subagent can settle the agent under a bare id.
    out.sort_by(|a, b| {
        a.owner
            .cmp(&b.owner)
            .then(a.delegated.cmp(&b.delegated))
            .then(a.path.cmp(&b.path))
    });
    out
}

/// Session id → name, from the live registry at `~/.claude/sessions`, keyed by pid.
pub fn registry_names(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let (Some(id), Some(name)) = (json["sessionId"].as_str(), json["name"].as_str())
            && !name.is_empty()
        {
            out.insert(id.to_string(), name.to_string());
        }
    }
    out
}

/// The name a transcript records for itself, for sessions the registry has
/// forgotten; stale after a rename. The quotes are backslash-escaped — the bytes
/// read `named this session \"home\"`. First occurrence wins.
fn named_in_transcript(text: &[u8]) -> Option<String> {
    let needle = b"named this session ";
    let mut start = find_at(text, needle, 0)? + needle.len();
    // Skip the opening quote in whichever form it takes.
    while matches!(text.get(start), Some(b'\\') | Some(b'"')) {
        start += 1;
    }
    let end = (start..text.len()).find(|&i| text[i] == b'\\' || text[i] == b'"')?;
    let name = std::str::from_utf8(&text[start..end]).ok()?;
    (!name.is_empty() && name.len() <= 40).then(|| name.to_string())
}

/// The name a session is going by NOW, from the `agent-name` line the CLI
/// re-appends. This, not the registry, is where a chosen name lives: every
/// registry entry now carries a name the CLI made up (`code-c4`). Last occurrence
/// wins. Anchored on the whole opening `{"type":"agent-name",` — quoted lines
/// inside a transcript are backslash-escaped — and the session id on the line
/// must be the transcript's own.
fn titled_in_transcript(text: &[u8], owner: &str) -> Option<String> {
    // The actor's order: the name it was given wins over the title, where the
    // console prefers the title — see [`reader::transcript::AS_ACTOR`].
    let written: Vec<Vec<u8>> = reader::transcript::AS_ACTOR
        .iter()
        .map(|line| reader::transcript::name_needle(line))
        .collect();
    written
        .iter()
        .find_map(|needle| last_titled(text, needle, owner))
}

/// The value on the last line opening with `needle`, when that line is `owner`'s.
fn last_titled(text: &[u8], needle: &[u8], owner: &str) -> Option<String> {
    let start = last_at(text, needle)? + needle.len();
    let end = find_at(text, b"\"", start)?;
    let line = find_at(text, b"\n", end).unwrap_or(text.len());
    // A line naming another session is not this session's name.
    find_at(&text[end..line], owner.as_bytes(), 0)?;
    let name = std::str::from_utf8(&text[start..end]).ok()?;
    (!name.is_empty() && name.len() <= 40).then(|| name.to_string())
}

/// The key holding the path, inside a tool call's `input` object.
const PATH_KEY: &[u8] = b"\"file_path\":\"";

/// The `Bash` commands on one transcript line, and the directory they ran in.
/// Parsed as JSON — a command is a string full of escapes — after a cheap byte
/// test, since one line in forty carries a `Bash` call. `None` for a `cwd` the
/// transcript does not record, so relative paths are unusable rather than guessed.
pub fn bash_calls(line: &[u8]) -> Option<(Option<String>, Vec<String>)> {
    let line = bash_calls_with_ids(line)?;
    Some((
        line.cwd,
        line.calls.into_iter().map(|call| call.command).collect(),
    ))
}

/// One `Bash` call: the command, the id its result will name, and what the
/// author said it was for.
#[derive(Debug, Clone)]
pub struct BashCall {
    pub id: String,
    pub command: String,
    /// The `description` the caller wrote beside the command: a CLAIM, never
    /// evidence about what ran (`docs/concept-model.md`). No reader may consult it to
    /// decide what a command did. `None` where the caller wrote none.
    pub description: Option<String>,
}

/// The `Bash` calls on one transcript line, with the directory they ran in.
#[derive(Debug, Clone)]
pub struct BashLine {
    pub cwd: Option<String>,
    /// When the line was written (RFC 3339, UTC) — the time of the CALL, not its
    /// result. Absent rather than defaulted: these rows are counted into days.
    pub at: Option<String>,
    pub calls: Vec<BashCall>,
}

/// As [`bash_calls`], keeping each call's id — the join to the result that
/// arrives on a later line.
pub fn bash_calls_with_ids(line: &[u8]) -> Option<BashLine> {
    find_at(line, b"\"name\":\"Bash\"", 0)?;
    let row: serde_json::Value = serde_json::from_slice(line).ok()?;
    let cwd = row["cwd"]
        .as_str()
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    let content = row["message"]["content"].as_array()?;
    let commands = content
        .iter()
        .filter(|item| item["type"] == "tool_use" && item["name"] == "Bash")
        .filter_map(|item| {
            Some(BashCall {
                id: item["id"].as_str().unwrap_or_default().to_string(),
                command: item["input"]["command"].as_str()?.to_string(),
                description: item["input"]["description"].as_str().map(str::to_string),
            })
        })
        .collect();
    Some(BashLine {
        cwd,
        at: row["timestamp"].as_str().map(str::to_string),
        calls: commands,
    })
}

/// The harness's own words for a call the user would not allow, anchored to the
/// front — see [`reader::doing::Verdict`].
const REFUSED: &[u8] = b"\"content\":\"The user doesn't want to proceed with this tool use";

/// What became of every call in one transcript, by id. `Ok` is kept: absent means
/// no result came back at all ([`reader::doing::Verdict::Unknown`]).
fn grep_matched_memories(text: &[u8], memory_root: &str) -> BTreeMap<String, usize> {
    // Built from `memory_root`, never a hardcoded `/memory/`, which silently
    // matches nothing in a test.
    let needle = format!("{}/", memory_root.trim_end_matches('/'));
    let needle = needle.as_bytes();
    const _: () = ();
    let mut out = BTreeMap::new();
    for line in text.split(|c| *c == b'\n') {
        if find_at(line, b"\"type\":\"tool_result\"", 0).is_none()
            || find_at(line, needle, 0).is_none()
        {
            continue;
        }
        // Counted ONCE per result, not per matching line: this records that the session
        // was shown the memory, not how loudly.
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut at = 0;
        while let Some(found) = find_at(line, needle, at) {
            let start = found + needle.len();
            let rest = &line[start..];
            let Some(end) = rest.iter().position(|c| !is_name_byte(*c)) else {
                break;
            };
            at = start + end;
            // The stem must be followed by exactly `.md:` — see above.
            if !rest[end..].starts_with(b".md:") {
                continue;
            }
            if end == 0 {
                continue;
            }
            if let Ok(name) = std::str::from_utf8(&rest[..end]) {
                seen.insert(name.to_string());
            }
        }
        for name in seen {
            *out.entry(name).or_default() += 1;
        }
    }
    out
}

/// A byte that may appear in a memory's canonical id (its filename stem).
fn is_name_byte(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'
}

fn outcomes(text: &[u8]) -> std::collections::HashMap<String, reader::doing::Verdict> {
    let mut out = std::collections::HashMap::new();
    for line in text.split(|c| *c == b'\n') {
        if let Some((call, verdict)) = tool_result(line) {
            out.insert(call, verdict);
        }
    }
    out
}

/// The `cd` targets each call's own output says the shell refused — a different
/// fact from the verdict: `cd nope; cat x` exits 0. See
/// [`reader::doing::refused_dirs`]. Cheap byte test first: a refusal is rare
/// (247 in the corpus), and the needle is the shell's own ending, since `cd: `
/// matches prose.
pub fn refusals(text: &[u8]) -> std::collections::HashMap<String, Vec<String>> {
    let mut out = std::collections::HashMap::new();
    for line in text.split(|c| *c == b'\n') {
        // The gate comes from the parser, not from here: a private needle hid zsh's
        // lower-cased wording and bash's `Not a directory`.
        if !reader::doing::may_hold_refusal(line)
            || find_at(line, b"\"type\":\"tool_result\"", 0).is_none()
        {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        let Some(items) = row["message"]["content"].as_array() else {
            continue;
        };
        for item in items {
            let Some(call) = item["tool_use_id"].as_str() else {
                continue;
            };
            // A result's content is a string on some rows and a list of blocks on others.
            // Not `to_string()` on the list — that re-escapes every newline.
            let said = match &item["content"] {
                serde_json::Value::String(text) => text.clone(),
                serde_json::Value::Array(blocks) => blocks
                    .iter()
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => continue,
            };
            let refused = reader::doing::refused_dirs(&said);
            if !refused.is_empty() {
                out.insert(call.to_string(), refused);
            }
        }
    }
    out
}

/// Whether this transcript exists only to name another conversation: the CLI
/// titles a conversation with a one-shot Haiku session — 307 of them against 13
/// real sessions. Both halves are needed: older CLIs wrote `ai-title` into the
/// working session's own transcript, so the line alone would delete the largest
/// sessions; a titler makes no tool call. The tool-call test runs FIRST, since a
/// session investigating this quotes the titling prompt into its own transcript.
fn titling(text: &[u8]) -> bool {
    if find_at(text, b"\"type\":\"tool_use\"", 0).is_some() {
        return false;
    }
    const TITLED: &[u8] = b"\"type\":\"ai-title\"";
    const ASKED: &[u8] = b"Below is part of a conversation between a person and a coding agent";
    find_at(text, TITLED, 0).is_some() || find_at(text, ASKED, 0).is_some()
}

/// Whether the tool call whose name begins at `at` did what it was asked. The
/// nearest `"id":"` behind the needle is this call's — a line can carry several.
/// An unreadable id is treated as completed rather than dropping real work.
fn call_completed(
    line: &[u8],
    at: usize,
    outcomes: &std::collections::HashMap<String, reader::doing::Verdict>,
) -> bool {
    let Some(id) = call_id(line, at) else {
        return true;
    };
    outcomes
        .get(id)
        .copied()
        .unwrap_or(reader::doing::Verdict::Unknown)
        .completed()
}

/// The `file_path` inside one tool call's input, if it has one. Not always the
/// first key — `Edit` serialises `replace_all` ahead of it, all 28,546 of them —
/// so it is looked up inside the object, bounded by `limit`.
fn path_in(line: &[u8], input: usize, limit: usize) -> Option<&str> {
    let key = find_at(line, PATH_KEY, input)?;
    if key >= limit {
        return None;
    }
    let start = key + PATH_KEY.len();
    let end = find_at(line, b"\"", start)?;
    std::str::from_utf8(&line[start..end]).ok()
}

/// The tool-use id of the call a needle landed inside, shared with
/// [`call_completed`] so the two cannot disagree.
fn call_id(line: &[u8], at: usize) -> Option<&str> {
    const ID: &[u8] = b"\"id\":\"";
    let start = crate::couse::last_at(&line[..at], ID).map(|pos| pos + ID.len())?;
    let end = find_at(line, b"\"", start)?;
    std::str::from_utf8(&line[start..end]).ok()
}

/// The call a `tool_result` line answers, and what became of it. Needles rather
/// than a parse: a result carries the command's whole output.
pub fn tool_result(line: &[u8]) -> Option<(String, reader::doing::Verdict)> {
    find_at(line, b"\"type\":\"tool_result\"", 0)?;
    const ID: &[u8] = b"\"tool_use_id\":\"";
    let start = find_at(line, ID, 0)? + ID.len();
    let end = start + line[start..].iter().position(|c| *c == b'"')?;
    let id = std::str::from_utf8(&line[start..end]).ok()?.to_string();
    let verdict = if find_at(line, REFUSED, 0).is_some() {
        reader::doing::Verdict::Rejected
    } else if find_at(line, b"\"is_error\":true", 0).is_some() {
        reader::doing::Verdict::Failed
    } else {
        reader::doing::Verdict::Ok
    };
    Some((id, verdict))
}

/// The first time each commit hash was mentioned, and by whom — the session that
/// made it. Earliest by TIMESTAMP, not by walk order, so two halves of a scan
/// merge with [`keep_earliest`] to the whole scan's answer (memview#1240).
pub type FirstSeen = BTreeMap<String, (String, String)>;

/// Fold one map of sightings into another, keeping the earlier of each. The
/// merge IS the ordering rule, written once.
pub fn keep_earliest(into: &mut FirstSeen, other: FirstSeen) {
    for (sha, (stamp, who)) in other {
        note_earliest(into, &sha, &stamp, &who);
    }
}

/// Record one sighting, keeping whichever is earlier — the single copy of the
/// comparison.
fn note_earliest(first: &mut FirstSeen, sha: &str, stamp: &str, who: &str) {
    match first.get(sha) {
        Some((seen, _)) if seen.as_str() <= stamp => {}
        _ => {
            first.insert(sha.to_string(), (stamp.to_string(), who.to_string()));
        }
    }
}

/// Full hashes by their seven-character prefix, for recognising a mention.
type ShaIndex = BTreeMap<String, Vec<String>>;

/// Note any commit hash this line mentions, keeping the earliest sighting.
fn note_hashes(
    line: &[u8],
    stamp: Option<&str>,
    index: &ShaIndex,
    name: &str,
    first: &mut FirstSeen,
) {
    let Some(stamp) = stamp else {
        return;
    };
    for candidate in crate::commits::hash_candidates(line) {
        let Some(shas) = index.get(&candidate[..crate::commits::SHORT]) else {
            continue;
        };
        // The whole candidate must prefix the hash, not just its first seven characters.
        for sha in shas.iter().filter(|sha| sha.starts_with(candidate)) {
            note_earliest(first, sha, stamp, name);
        }
    }
}

/// Whether a transcript line is somebody typing, rather than the machinery
/// answering the agent's own call: of 492,124 `user` lines, 39,973 are typing.
/// The needle is the JSON key `"type":"tool_result"`, not the bare word — a
/// prompt that DISCUSSES tool results merged 17 episodes — and inside a JSON
/// string every quote is escaped, so those bytes are only ever structure.
pub fn is_prompt(line: &[u8]) -> bool {
    field(line, "type") == Some(b"user") && find_at(line, b"\"type\":\"tool_result\"", 0).is_none()
}

/// Count one transcript's tool calls into `agent`, and note the days.
#[allow(clippy::too_many_arguments)]
fn scan_transcript(
    text: &[u8],
    code_root: &str,
    memory_root: &str,
    home: &str,
    index: &ShaIndex,
    first: &mut FirstSeen,
    agent: &mut Agent,
    seen: &mut DaysSeen,
    log: &mut reader::doing::Log,
    effects: &mut reader::effects::Log,
    resume: Option<&reader::watermark::Resume>,
) {
    // Borrowed field by field: the compiler cannot see the two counters are disjoint.
    let Agent {
        name: agent_name,
        reads: agent_reads,
        writes: agent_writes,
        memories,
        paths,
        shell_paths,
        remote_paths,
        first: earliest,
        last,
        ..
    } = agent;
    // What became of each call — read ahead, because the answer is always below
    // the question.
    let outcomes = outcomes(text);
    // A whole-transcript pass: the evidence is a tool RESULT, which the walk below
    // does not visit.
    for (name, hits) in grep_matched_memories(text, memory_root) {
        memories.entry(name).or_default().grep_matches += hits;
    }
    // What the shell said it could not do — see [`refusals`]. A refused `cd` must
    // not be applied to the walk below.
    let refusals = refusals(text);
    // Built once per transcript rather than once per line — the needles are
    // fixed and the corpus is millions of lines.
    let needles: Vec<(String, &str, Option<bool>)> = TOOLS
        .iter()
        .map(|(tool, role)| (format!("\"name\":\"{tool}\",\"input\":{{"), *tool, *role))
        .collect();
    // Per transcript, or the last instruction of one session adopts the first rows
    // of the next file. But a RESUMED read must not reset: reopening here and
    // clearing on the next statement threw the carried episode away (memview#1240).
    match resume {
        None => log.open_transcript(),
        Some(open) => log.reopen(open.episode, open.prompt.clone()),
    }
    for line in text.split(|&c| c == b'\n') {
        if line.is_empty() {
            continue;
        }
        if is_prompt(line) {
            log.begin_episode(agent_name);
        }
        let mut day = None;
        let stamp = field(line, "timestamp").and_then(|t| std::str::from_utf8(t).ok());
        if let Some(stamp) = stamp {
            if earliest.is_empty() || stamp < earliest.as_str() {
                *earliest = stamp.to_string();
            }
            if stamp > last.as_str() {
                *last = stamp.to_string();
            }
            day = day_number(stamp);
        }
        // Attribution happens after every transcript has been read: "first" is a
        // claim about all of them.
        if !index.is_empty() {
            note_hashes(line, stamp, index, agent_name, first);
        }
        // What the shell did, beside what the tools did — a new dimension, not a
        // correction. A command the grammar cannot read contributes nothing;
        // `shell-report` measures that.
        if let Some((call, verdict)) = tool_result(line) {
            log.resolve(&call, verdict);
            effects.resolve(&call, verdict);
        }
        // The miner takes its time from the row it is already walking, so the
        // call's own stamp is not needed here.
        if let Some(BashLine { cwd, calls, .. }) = bash_calls_with_ids(line) {
            // The description is deliberately not read: the miner is a static analysis of
            // what ran. See [`BashCall::description`].
            for BashCall {
                id: call, command, ..
            } in calls
            {
                let Ok(parsed) = reader::project::read(&command) else {
                    continue;
                };
                // Traced, because the effects artefact shows the command a claim rests on and
                // only a `Step` carries it.
                let found = reader::shell_files::trace_knowing(
                    &parsed,
                    cwd.as_deref(),
                    home,
                    refusals.get(&call).map_or(&[][..], Vec::as_slice),
                );
                // What the text required, met with what the call returned: the timeline row
                // goes in whatever happened, but a path reaches somebody's name only when the
                // command that opened it certainly ran.
                let verdict = outcomes
                    .get(&call)
                    .copied()
                    .unwrap_or(reader::doing::Verdict::Unknown);
                // One row per kind of work in the turn: `sed` over four files is one edit.
                let mut kinds: BTreeMap<&str, u32> = BTreeMap::new();
                for activity in &found.activities {
                    if activity.is_work() {
                        *kinds.entry(activity.label()).or_default() += 1;
                    }
                }
                if let Some(minute) = stamp.and_then(reader::doing::minute) {
                    let project = cwd.as_deref().and_then(|dir| project_of(dir, code_root));
                    // One host or none: a turn that reached two machines is
                    // rare enough that naming the first is honest and naming
                    // both would need a row shape nothing else wants.
                    let host = found.remote.first().map(|use_| use_.host.clone());
                    for (kind, n) in kinds {
                        log.push(reader::doing::Work {
                            call: &call,
                            agent: agent_name,
                            project: project.as_deref(),
                            host: host.as_deref(),
                            kind,
                            n,
                            minute,
                        });
                    }
                }
                // What each command in the call did to which file — the evidence
                // under the timeline row above, keyed by the same
                // `(agent, minute)` so opening a turn is a filter, not a join.
                if let Some(minute) = stamp.and_then(reader::doing::minute) {
                    for step in &found.steps {
                        let command = step.argv.join(" ");
                        if command.is_empty() {
                            continue;
                        }
                        let searched = match &step.op {
                            Some(reader::shell_ops::Op::Search { pattern, .. })
                                if !pattern.is_empty() =>
                            {
                                Some(pattern.as_str())
                            }
                            _ => None,
                        };
                        let mut effect = |did, path, pattern, host| {
                            effects.push(reader::effects::Effect {
                                call: &call,
                                agent: agent_name,
                                minute,
                                did,
                                path,
                                pattern,
                                host,
                                command: &command,
                                reached: step.reached,
                            });
                        };
                        for used in &step.files {
                            let did = match (used.write, searched) {
                                (true, _) => reader::effects::Did::Wrote,
                                (false, Some(_)) => reader::effects::Did::Searched,
                                (false, None) => reader::effects::Did::Read,
                            };
                            effect(did, Some(used.path.as_str()), searched, None);
                        }
                        for used in &step.away {
                            let did = if used.write {
                                reader::effects::Did::Wrote
                            } else {
                                reader::effects::Did::Read
                            };
                            effect(did, Some(used.path.as_str()), searched, Some(&used.host));
                        }
                        // The admissions travel too: a turn that used a file nobody can name is not a
                        // turn that used none.
                        for pattern in &step.bounded {
                            effect(
                                reader::effects::Did::Unnamed,
                                Some(pattern.as_str()),
                                None,
                                None,
                            );
                        }
                        for _ in &step.unnamed {
                            effect(reader::effects::Did::Unnamed, None, None, None);
                        }
                        // A third vector, and skipping it lost 1,019 rows in silence (memview#1458).
                        for locus in &step.located {
                            effect(
                                reader::effects::Did::Located,
                                Some(locus.as_str()),
                                None,
                                None,
                            );
                        }
                    }
                }
                // A refusal drops the whole call; everything else is recorded under one of two
                // claims, never thrown away for being unsure.
                for used in found
                    .files
                    .into_iter()
                    .filter(|_| verdict != reader::doing::Verdict::Rejected)
                {
                    let certain = verdict.admits(used.reached);
                    let Some(rel) = relative_to(&used.path, code_root).filter(|p| attributable(p))
                    else {
                        // The corpus is outside the code root, so a `grep` over `memory/` counted for
                        // NOTHING here while the tool-call site had a `memory_of` arm — and searching
                        // the directory is one of only two ways a demoted memory is ever reached (#822).
                        if let Some(memory) = memory_of(&used.path, memory_root) {
                            if let Some(day) = day {
                                let days = if used.write {
                                    &mut seen.memory_edits
                                } else {
                                    &mut seen.memory_reads
                                };
                                days.entry(memory.clone()).or_default().insert(day);
                            }
                            // Four-way where the tool call is two-way: a command only may have opened it.
                            let use_ = memories.entry(memory).or_default();
                            match (certain, used.write) {
                                (true, true) => use_.edits += 1,
                                (true, false) => use_.reads += 1,
                                (false, true) => use_.maybe_edits += 1,
                                (false, false) => use_.maybe_reads += 1,
                            }
                        }
                        continue;
                    };
                    // The day counts, even though the count does not: recency decides which project
                    // an agent is listed under, and a session editing through `sed` was present.
                    if let (Some(project), Some(day)) = (project_of(&used.path, code_root), day) {
                        let days = if used.write {
                            &mut seen.writes
                        } else {
                            &mut seen.reads
                        };
                        days.entry(project).or_default().insert(day);
                    }
                    let use_ = shell_paths.entry(rel).or_default();
                    match (certain, used.write) {
                        (true, true) => use_.edits += 1,
                        (true, false) => use_.reads += 1,
                        (false, true) => use_.maybe_edits += 1,
                        (false, false) => use_.maybe_reads += 1,
                    }
                }
                // Work on another machine, kept under its host, with no code-root filter.
                for used in found
                    .remote
                    .into_iter()
                    .filter(|_| verdict != reader::doing::Verdict::Rejected)
                {
                    if !remotely_attributable(&used.path) {
                        continue;
                    }
                    let use_ = remote_paths
                        .entry(format!("{}:{}", used.host, used.path))
                        .or_default();
                    if used.write {
                        use_.edits += 1;
                    } else {
                        use_.reads += 1;
                    }
                }
            }
        }
        // A line can carry more than one tool call; a batched turn that opens six files
        // is six reads.
        for (head, tool, role) in &needles {
            let mut from = 0;
            while let Some(at) = find_at(line, head.as_bytes(), from) {
                let input = at + head.len();
                from = input;
                // Bounded by the next tool call so a call carrying no path cannot borrow the
                // following call's. A payload cannot forge the marker: its quotes are escaped.
                let limit = find_at(line, b"\"name\":\"", input).unwrap_or(line.len());
                // The timeline row goes in whatever the call returned; file attribution is
                // gated below. Pushed before the path is known, because a `Grep` never has one
                // and is still work.
                if let Some(minute) = stamp.and_then(reader::doing::minute)
                    && let Some(activity) = reader::activity::Activity::of_tool(tool)
                    && let Some(call) = call_id(line, at)
                {
                    let project =
                        path_in(line, input, limit).and_then(|p| project_of(p, code_root));
                    log.push(reader::doing::Work {
                        call,
                        agent: agent_name,
                        project: project.as_deref(),
                        host: None,
                        kind: activity.label(),
                        n: 1,
                        minute,
                    });
                }
                let Some(is_write) = role else {
                    continue; // no path of its own to attribute
                };
                let is_write = *is_write;
                let (counter, days) = if is_write {
                    (&mut *agent_writes, &mut seen.writes)
                } else {
                    (&mut *agent_reads, &mut seen.reads)
                };
                if !call_completed(line, at, &outcomes) {
                    continue;
                }
                // A tool call that failed did nothing: 990 failed `Edit`s and 289 `Write`s were
                // counted as changes to files they never altered.
                let Some(path) = path_in(line, input, limit) else {
                    continue;
                };
                {
                    if let Some(project) = project_of(path, code_root) {
                        *counter.entry(project.clone()).or_insert(0) += 1;
                        if let Some(day) = day {
                            days.entry(project).or_default().insert(day);
                        }
                        if let Some(rel) = relative_to(path, code_root).filter(|p| attributable(p))
                        {
                            let use_ = paths.entry(rel).or_default();
                            if is_write {
                                use_.edits += 1;
                            } else {
                                use_.reads += 1;
                            }
                        }
                    } else if let Some(memory) = memory_of(path, memory_root) {
                        // The day, beside the count: a memory opened forty times in one afternoon
                        // outranks one opened daily for a fortnight, on counts alone.
                        if let Some(day) = day {
                            let days = if is_write {
                                &mut seen.memory_edits
                            } else {
                                &mut seen.memory_reads
                            };
                            days.entry(memory.clone()).or_default().insert(day);
                        }
                        let use_ = memories.entry(memory).or_default();
                        if is_write {
                            use_.edits += 1;
                        } else {
                            use_.reads += 1;
                        }
                    }
                }
            }
        }
    }
}

/// Where a scan reads from. Bundled because passing five paths positionally is
/// how `memory_root` ends up handed to `code_root`.
#[derive(Clone, Copy)]
pub struct Roots<'a> {
    pub projects: &'a Path,
    pub sessions: &'a Path,
    pub code_root: &'a str,
    pub memory_root: &'a str,
    pub home: &'a str,
}

/// What a caller actually wants out of a scan: `memory-rank` referenced commit
/// data zero times and paid 4.4s of git walk for it on every run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    /// Attribute commits to the agents who mentioned them, walking every repository's
    /// git log. Off means the commit fields are EMPTY, so such a scan must never be
    /// written to `agents.json`.
    pub commits: bool,
}

impl Needs {
    /// Everything the artefacts hold. What the miner asks for.
    pub const EVERYTHING: Self = Self { commits: true };
    /// The roster and the day sets — what the memory tools read.
    pub const MEMORIES: Self = Self { commits: false };
}

/// The artefacts a previous run wrote, plus the fold state they could not carry.
/// See [`crate::mine::Carried`].
pub struct Resumed {
    /// Watermarks and the fold state the artefacts cannot hold, INCLUDING the raw
    /// roster: `agents.json`'s has renames applied, and re-seeding from it toggles
    /// any path in a rename cycle.
    pub carried: crate::mine::Carried,
    /// The timeline as last written.
    pub doing: reader::doing::Doing,
    /// The evidence under the timeline, as last written.
    pub effects: reader::effects::Effects,
}

/// Read the whole corpus from scratch — what [`Plan::Full`](reader::watermark::Plan::Full)
/// falls back to.
pub fn scan(
    projects_root: &Path,
    sessions_dir: &Path,
    code_root: &str,
    memory_root: &str,
    home: &str,
    generated: &str,
) -> Result<Agents> {
    scan_resumed(
        Roots {
            projects: projects_root,
            sessions: sessions_dir,
            code_root,
            memory_root,
            home,
        },
        generated,
        None,
        Needs::EVERYTHING,
    )
    .map(|(agents, _)| agents)
}

/// Read only what has changed since `from`, and return the state the next run
/// resumes from. All-or-nothing: the artefacts carry no per-transcript
/// provenance, so one unprovable append forces a whole re-read. An unchanged
/// transcript is not re-read AND not forgotten: its watermark is carried.
pub fn scan_resumed(
    at: Roots<'_>,
    generated: &str,
    from: Option<Resumed>,
    needs: Needs,
) -> Result<(Agents, crate::mine::Carried)> {
    let Roots {
        projects: projects_root,
        sessions: sessions_dir,
        code_root,
        memory_root,
        home,
    } = at;
    let names = registry_names(sessions_dir);
    let held = from.unwrap_or_else(|| Resumed {
        carried: crate::mine::Carried::default(),
        doing: reader::doing::Doing::default(),
        effects: reader::effects::Effects::default(),
    });
    // Seeded from the CARRIED raw roster, never `agents.json` — the rename map is
    // not idempotent. See [`crate::mine::Carried::agents`].
    let mut by_name: BTreeMap<String, Agent> = held
        .carried
        .agents
        .iter()
        .cloned()
        .map(|a| (a.name.clone(), a))
        .collect();
    // The timeline, built across every transcript and frozen at the end.
    let mut log = reader::doing::Log::resume(held.doing);
    // The evidence under the timeline, built in the same pass and written to its
    // own file for the same reasons.
    let mut effects = reader::effects::Log::resume(held.effects);
    let mut days: BTreeMap<String, DaysSeen> = held.carried.days;
    // Read before the transcripts, since recognising a hash needs the set to look
    // for. Empty when the caller does not need attribution — see [`Needs`].
    let history = if needs.commits {
        crate::commits::all(Path::new(code_root))?
    } else {
        Vec::new()
    };
    let mut index: ShaIndex = BTreeMap::new();
    for commit in &history {
        if commit.sha.len() >= crate::commits::SHORT {
            index
                .entry(commit.sha[..crate::commits::SHORT].to_string())
                .or_default()
                .push(commit.sha.clone());
        }
    }
    let mut first_seen: FirstSeen = held.carried.first_seen;
    // "Now" is the mine's own stamp, so the weights are a property of the artefact.
    let today = day_number(generated).unwrap_or(0);

    std::fs::metadata(projects_root)
        .with_context(|| format!("reading {}", projects_root.display()))?;
    // The name an owner settled on, carried because a session is named in the HEAD
    // of its transcript and a resumed run reads only the tail.
    let mut resolved: BTreeMap<String, String> = held.carried.resolved;

    let found = transcripts_under(projects_root);
    let present: Vec<std::path::PathBuf> = found.iter().map(|t| t.path.clone()).collect();
    let plan = reader::watermark::plan(&held.carried.marks, &present);
    // A full re-read must also DISCARD what was carried, or everything is doubled.
    // Zero marks beside a non-empty roster is incoherent — `plan` would call every
    // transcript new and add it on top — and is treated as a full re-mine.
    let incoherent = held.carried.marks.is_empty() && !held.carried.agents.is_empty();
    if incoherent {
        eprintln!(
            "⚠ resume state has no watermarks but a roster of {} — re-reading everything",
            held.carried.agents.len()
        );
    }
    if incoherent || matches!(plan, reader::watermark::Plan::Full { .. }) {
        by_name.clear();
        log = reader::doing::Log::default();
        effects = reader::effects::Log::default();
        days.clear();
        first_seen.clear();
        resolved.clear();
    }
    let mut marks: BTreeMap<String, reader::watermark::Resume> = BTreeMap::new();

    for transcript in &found {
        let key = transcript.path.to_string_lossy().into_owned();
        // Where this run starts in this file: the beginning unless the plan proved the
        // prefix untouched.
        let resume = match &plan {
            reader::watermark::Plan::Full { .. } => None,
            reader::watermark::Plan::Resume { tails, .. } => match tails.get(&key) {
                Some(held) => Some(held.clone()),
                None => match held.carried.marks.get(&key) {
                    // Unchanged since the last run: nothing to read, but its
                    // watermark has to survive into the next run's state.
                    Some(unchanged) => {
                        marks.insert(key, unchanged.clone());
                        continue;
                    }
                    // Never seen before: read it whole, carrying nothing.
                    None => None,
                },
            },
        };
        let start = resume.as_ref().map(|r| r.mark.read_to).unwrap_or(0);
        let Ok(text) = read_from(&transcript.path, start) else {
            continue;
        };
        // Only meaningful for a whole read: a tail does not contain the header.
        if start == 0 && titling(&text) {
            continue;
        }
        // The open episode is handed to `scan_transcript`, which opens the log itself.
        // The name it goes by now, then the registry, then the reminder, then the id:
        // the registry now holds a CLI-invented name (`code-c4`), so it cannot come
        // first — see [`titled_in_transcript`]. An unnamed session shows its id rather
        // than joining an "unknown" bucket.
        let name = resolved
            .entry(transcript.owner.clone())
            .or_insert_with(|| {
                // Only a session's own transcript names it; quoting a name is not being called one.
                (!transcript.delegated)
                    .then(|| titled_in_transcript(&text, &transcript.owner))
                    .flatten()
                    .or_else(|| names.get(&transcript.owner).cloned())
                    .or_else(|| {
                        (!transcript.delegated)
                            .then(|| named_in_transcript(&text))
                            .flatten()
                    })
                    .unwrap_or_else(|| transcript.owner.clone())
            })
            .clone();
        let agent = by_name.entry(name.clone()).or_insert_with(|| Agent {
            name,
            ..Agent::default()
        });
        // The owner id, so a delegated transcript records the session that dispatched it.
        agent.sessions.insert(transcript.owner.clone());
        // Counted once per transcript, not once per READ: a resumed run reads the tail
        // of a file already counted. `resume.is_none()` is "reading from the start".
        if resume.is_none() {
            if transcript.delegated {
                agent.delegated += 1;
            } else {
                agent.transcripts += 1;
            }
        }
        scan_transcript(
            &text,
            code_root,
            memory_root,
            home,
            &index,
            &mut first_seen,
            agent,
            days.entry(agent.name.clone()).or_default(),
            &mut log,
            &mut effects,
            resume.as_ref(),
        );
        // Taken AFTER the read, at the length consumed, with whatever episode is still
        // open: the two must describe the same instant.
        if let Some(mark) = reader::watermark::observe(&transcript.path) {
            let (episode, prompt) = log.open_episode();
            marks.insert(
                key,
                reader::watermark::Resume {
                    mark,
                    episode,
                    prompt,
                },
            );
        }
    }

    // Memory days are unioned across agents, where project days are not: a memory
    // is the corpus's, and two agents opening it on one day is one day of it being live.
    type DaySet = std::collections::BTreeSet<i64>;
    let mut memory_read_days: BTreeMap<String, DaySet> = BTreeMap::new();
    let mut memory_edit_days: BTreeMap<String, DaySet> = BTreeMap::new();
    for (name, seen) in &days {
        // Sets, because two agents opening one memory on the same day is one day
        // of it being live — appending to a list would count it twice and make
        // the shared memories look busier than the depended-on ones.
        for (memory, when) in &seen.memory_reads {
            memory_read_days
                .entry(memory.clone())
                .or_default()
                .extend(when);
        }
        for (memory, when) in &seen.memory_edits {
            memory_edit_days
                .entry(memory.clone())
                .or_default()
                .extend(when);
        }

        let Some(agent) = by_name.get_mut(name) else {
            continue;
        };
        for (project, when) in &seen.reads {
            agent
                .recent_reads
                .insert(project.clone(), recency(when, today));
        }
        for (project, when) in &seen.writes {
            agent
                .recent_writes
                .insert(project.clone(), recency(when, today));
        }
    }

    // Every transcript has been read, so "who saw this hash first" has an answer;
    // a commit nobody mentioned goes to nobody. Cleared first, because attribution
    // is RECOMPUTED from the whole git log: without the reset a resumed mine
    // reported exactly double, and the fixtures carry no git history to show it.
    for agent in by_name.values_mut() {
        agent.commits = 0;
        agent.commit_lines.clear();
    }
    let mut unattributed = 0usize;
    for commit in &history {
        let Some((_, who)) = first_seen.get(&commit.sha) else {
            unattributed += 1;
            continue;
        };
        let Some(agent) = by_name.get_mut(who) else {
            unattributed += 1;
            continue;
        };
        agent.commits += 1;
        for file in &commit.files {
            if !attributable(&file.path) {
                continue;
            }
            let delta = agent.commit_lines.entry(file.path.clone()).or_default();
            delta.added += file.added;
            delta.deleted += file.deleted;
            delta.commits += 1;
        }
    }

    // A renamed file has been filed under both names by tools, shell and git; git
    // alone knows they are one file, so its answer is applied here, at the end.
    let renames = renames(&history);
    let mut agents: Vec<Agent> = by_name.into_values().collect();
    // Taken BEFORE `rename_keys`: what is carried is the raw accumulation.
    let raw_roster = agents.clone();
    for agent in &mut agents {
        agent.paths = rename_keys(std::mem::take(&mut agent.paths), &renames);
        agent.shell_paths = rename_keys(std::mem::take(&mut agent.shell_paths), &renames);
        agent.commit_lines = rename_keys(std::mem::take(&mut agent.commit_lines), &renames);
    }
    agents.sort_by_key(|a| {
        let total: usize = a.reads.values().sum::<usize>() + a.writes.values().sum::<usize>();
        (std::cmp::Reverse(total), a.name.clone())
    });
    let memory_days = memory_read_days
        .keys()
        .chain(memory_edit_days.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|memory| {
            let days = |from: &BTreeMap<String, DaySet>| {
                from.get(&memory)
                    .map(|set| set.iter().copied().collect())
                    .unwrap_or_default()
            };
            let use_ = MemoryDays {
                reads: days(&memory_read_days),
                edits: days(&memory_edit_days),
            };
            (memory, use_)
        })
        .collect();

    // Finished BEFORE the resume state is built, because finishing renumbers the
    // episodes; saved unremapped, every carried mark would name a different
    // instruction and read as a plausible timeline.
    let (doing, episodes) = log.finish_canonical(generated);
    for mark in marks.values_mut() {
        mark.episode = mark.episode.and_then(|old| episodes.get(&old).copied());
    }
    let carried = crate::mine::Carried {
        generated: generated.to_string(),
        marks,
        resolved,
        first_seen,
        days,
        agents: raw_roster,
    };
    Ok((
        Agents {
            doing,
            effects: effects.finish(generated),
            generated: generated.to_string(),
            commits: history.len(),
            unattributed,
            renames,
            memory_days,
            agents,
        },
        carried,
    ))
}

/// A transcript from `start` to its end. Seeks rather than reading and slicing:
/// the files are gigabytes.
fn read_from(path: &Path, start: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Where each old path ended up, following a chain of renames. Newest-first
/// history, so the chain is walked forward from each entry; bounded, since git
/// will record a rename cycle.
fn renames(history: &[crate::commits::Commit]) -> BTreeMap<String, String> {
    let mut step: BTreeMap<String, String> = BTreeMap::new();
    for commit in history {
        for file in &commit.files {
            if let Some(was) = &file.was
                && was != &file.path
            {
                // Newest first, so an earlier commit's rename is the older fact.
                step.entry(was.clone()).or_insert_with(|| file.path.clone());
            }
        }
    }
    step.keys()
        .map(|from| {
            let mut to = from;
            for _ in 0..8 {
                match step.get(to) {
                    Some(next) if next != from => to = next,
                    _ => break,
                }
            }
            (from.clone(), to.clone())
        })
        .filter(|(from, to)| from != to)
        .collect()
}

/// Re-key a path map onto the names those files now have.
fn rename_keys<T: Default + Merge>(
    map: BTreeMap<String, T>,
    renames: &BTreeMap<String, String>,
) -> BTreeMap<String, T> {
    let mut out: BTreeMap<String, T> = BTreeMap::new();
    for (path, value) in map {
        let key = renames.get(&path).cloned().unwrap_or(path);
        out.entry(key).or_default().merge(value);
    }
    out
}

/// Adding one file's figures to another's, for re-keying two names onto one file.
pub trait Merge {
    fn merge(&mut self, other: Self);
}

impl Merge for MemoryUse {
    fn merge(&mut self, other: Self) {
        // Destructured so a new field cannot quietly go unmerged — a counter missing
        // here is reset to zero for every entry, which is how `maybe_reads` was first lost.
        let MemoryUse {
            reads,
            edits,
            maybe_reads,
            maybe_edits,
            grep_matches,
        } = other;
        self.reads += reads;
        self.edits += edits;
        self.maybe_reads += maybe_reads;
        self.maybe_edits += maybe_edits;
        self.grep_matches += grep_matches;
    }
}

impl Merge for LineDelta {
    fn merge(&mut self, other: Self) {
        self.added += other.added;
        self.deleted += other.deleted;
        self.commits += other.commits;
    }
}

//! Which named Claude session works on which part of the codebase.
//!
//! Several sessions run in parallel, each named for what it does — `recall`,
//! `health`, `tumor`, `observe`. That naming is a claim, and this is the
//! evidence for or against it: what each one actually opened and actually
//! changed, counted per project directory.
//!
//! **The signal is the file paths of tool calls, not `cwd` and not any text.**
//! `cwd` says where a session was started and barely moves; text is hopeless,
//! because MEMORY.md names every project and is injected into every session, so
//! grepping for a project name matches nearly everything. What a session
//! *opened* and *wrote* cannot be faked by injected context, and it is the
//! record of work rather than of intent.
//!
//! **Reads and writes are counted apart, and that distinction is the point.** A
//! session that reads a repository is consulting it; one that writes there is
//! responsible for it. `health` reads the `pippijn` monorepo more than anything
//! else while doing its writing in `health` — reporting one number would call it
//! a monorepo session, which is not what it is.
//!
//! **Where an agent works is decided by recent days present, not by lifetime
//! file counts** — see [`recency`]. A session is renamed as its job changes, and
//! the name is a claim about what it is doing *now*, so its history has to be
//! weighted the same way.
//!
//! Only names, project names and integers leave this module — the same rule the
//! rest of the mining follows, for the same reason
//! (see the module docs on [`crate::couse`]).
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::couse::{field, find_at, last_at};

/// One named session, and where its work actually landed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// The session ids filed under this name.
    ///
    /// More than one when a name has been reused, and the reason this is kept
    /// at all: every memory records the `originSessionId` that wrote it, which
    /// is a raw uuid until something can say which agent that was. Without this
    /// the corpus and the roster are two datasets about the same sessions with
    /// no join between them.
    #[serde(default)]
    pub sessions: BTreeSet<String>,
    /// Files opened, per project directory. Lifetime totals, undecayed — the
    /// honest record of what happened, and what the totals line reports.
    pub reads: BTreeMap<String, usize>,
    /// Files written or edited, per project directory. Lifetime, undecayed.
    pub writes: BTreeMap<String, usize>,
    /// Every file this agent touched under the code root, keyed by its path
    /// relative to that root — `xinutec-infra/plan/backup.dhall`.
    ///
    /// [`reads`](Self::reads) and [`writes`](Self::writes) keep only the first
    /// segment, which answers "which repository" and refuses everything finer.
    /// That refusal is what made *who built the Dhall reconciler* unanswerable:
    /// its 34 commits live in `xinutec-infra/plan/`, filed under `xinutec-infra`
    /// beside firewall tweaks and backup scripts, and the whole `pippijn`
    /// monorepo lands in one bucket. Keeping the path is what lets a subtree, a
    /// filename or an extension be asked about.
    ///
    /// Cheap, because real work is not many files: about 7,300 distinct paths
    /// across the entire history. So there is no cap here, and therefore no
    /// silent truncation to explain. Build output and dependency trees are left
    /// out — see [`attributable`].
    #[serde(default)]
    pub paths: BTreeMap<String, MemoryUse>,
    /// The same, for files used by shell commands rather than by tool calls —
    /// `sed -i`, `cp`, a `>` redirect. Keyed identically, so the two are unioned
    /// by [`Agents::who_works_on`] at query time.
    ///
    /// **A separate map, and deliberately so.** Folding shell use into
    /// [`paths`](Self::paths) would move every existing figure on the agents
    /// page at once, and would retroactively reward the habit of editing through
    /// `sed` over the habit of editing through `Edit` — a change to what the
    /// numbers have always meant, made silently. Kept apart, the old numbers go
    /// on meaning what they meant and the new evidence is visible as its own
    /// claim.
    ///
    /// Not a small addition: two thirds of the fleet's shell commands touch
    /// files, and the `Write`/`Edit` miner sees none of it.
    ///
    /// "Shell" means *the Bash call*, not only the shell language: the Python of
    /// a `python3 - <<'PY'` heredoc lands here too, and it is the single biggest
    /// file-changer in the corpus at 3,048 writes.
    #[serde(default)]
    pub shell_paths: BTreeMap<String, MemoryUse>,
    /// Lines committed, per repo-relative path — the third dimension, and the
    /// only one that measures *size* rather than counting operations.
    ///
    /// A `Write` of three hundred lines and a one-character `Edit` are both
    /// worth 1 to the maps above. This is what tells them apart, and it is also
    /// the only evidence that survived review: an experiment written and thrown
    /// away leaves tool calls behind and leaves nothing here.
    ///
    /// Attributed by [`crate::commits`]'s earliest-mention rule, so a commit
    /// nobody's transcript mentions is counted nowhere and reported as
    /// unattributed rather than assigned to a guess.
    #[serde(default)]
    pub commit_lines: BTreeMap<String, LineDelta>,
    /// Files used on **other machines**, keyed `host:/absolute/path` — where
    /// this agent's work lands when it is not on this one.
    ///
    /// A fourth dimension, and the only one about somewhere else. It comes
    /// entirely from `ssh`/`kubectl exec` payloads, so it is shell-derived by
    /// construction; git cannot attribute it, because those commits are made on
    /// the remote host and never appear in a repository here.
    ///
    /// Kept apart from [`paths`](Self::paths) for the obvious reason: a path
    /// under `/etc/nixos` exists on odin and not here, and merging the two would
    /// make every local answer wrong.
    #[serde(default)]
    pub remote_paths: BTreeMap<String, MemoryUse>,
    /// Commits attributed to this agent, across every repository.
    #[serde(default)]
    pub commits: usize,
    /// Which memories this agent works with, keyed by memory name.
    ///
    /// The companion to `reads`/`writes` and a different question: those say
    /// where an agent is *responsible*, this says what it has *consulted*. For
    /// handing out a task the second is often the better evidence — territory
    /// says who owns a repository, and this says who has read the rules that
    /// govern it.
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
/// the file.
///
/// **Counted from the tool call's own `file_path`, not from the memory being
/// named.** Counting names was tried first and is unusable: the co-use miner's
/// reasoning for preferring mentions — that opens are too sparse — is about
/// *pairs*, where a turn must name two memories at once, and it does not
/// transfer to one agent's familiarity with one memory, where opens are
/// plentiful. What mentions actually measure here is re-injected context: a
/// single sentence naming `feedback_weighted_over_binary` recurred 3,370 times
/// in one session's transcript, swamping every real signal. Per-turn dedup
/// would not have saved it, because the injection is per turn.
///
/// Reads and edits stay apart because they answer different questions: who went
/// and looked it up, and who is maintaining it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryUse {
    /// Times this agent opened the memory with `Read`.
    pub reads: usize,
    /// Times this agent wrote or edited it — the strongest claim to it.
    pub edits: usize,
    /// Times a command that **may** have opened it did.
    ///
    /// ⚠ **A weaker claim, kept apart rather than merged or discarded.** A shell
    /// command after `&&` runs only if what came before it worked, and one exit
    /// status for a whole script often cannot say whether it did — 19,256 file
    /// uses in the corpus. Counting those as fact overstates the record and
    /// dropping them understates it; both were tried, and the truth is that they
    /// are a different kind of evidence.
    ///
    /// Always zero for tool calls, which are atomic: an `Edit` either replaced
    /// the text or changed nothing, and its result says which.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub maybe_reads: usize,
    /// Times a command that **may** have changed it did. See
    /// [`maybe_reads`](Self::maybe_reads).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub maybe_edits: usize,
}

/// Kept out of the artefact when nothing is uncertain, which is most entries —
/// the file is read over a VPN and these two fields would otherwise be written
/// as zeroes on every path anyone ever opened.
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What one agent's commits did to one file.
///
/// Added and deleted stay apart: a rewrite that removes 181 lines and adds 594
/// is not the same work as writing 413 from nothing, and one net figure would
/// call them equal. Deletion is work too — the largest single change to the
/// Dhall configs in this corpus is the removal of a file that had rotted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// **File changes committed, not commits.** One commit touching four
    /// matching files counts four, because the per-path record does not keep
    /// which commit was which and a distinct count cannot be recovered from it.
    /// Named for what it measures rather than for what a reader might assume.
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
    /// The names this file used to have, newest last.
    ///
    /// Empty for the ordinary case. Present, it is the reason a file created
    /// last week can carry a year of history — and without saying so, that
    /// history reads as a counting bug.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub was: Vec<String>,
    /// Every use, tool call and shell command together.
    pub reads: usize,
    pub edits: usize,
    /// How much of the above came from a `Bash` call rather than from `Write`
    /// and `Edit` — including the Python and the other machines' shells inside
    /// one. Reported so the evidence can be checked: a file with forty changes
    /// and no tool edits is not a mistake, it is somebody working through `sed`
    /// or a `python3 -` heredoc, and without this split there is no way to see
    /// that.
    pub shell_reads: usize,
    pub shell_edits: usize,
    /// The machine this file is on, when it is not this one. `None` is local.
    ///
    /// Remote use is shell-derived by construction — it can only come from an
    /// `ssh` or `kubectl exec` payload — so `shell_reads`/`shell_edits` already
    /// say where the numbers came from, and this says where the *file* is.
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
    /// When this was mined, ISO-8601 — the artefact's own account of its age,
    /// because an mtime records the last copy rather than the last derivation.
    #[serde(default)]
    pub generated: String,
    /// Commits found under the code root, and how many of them no transcript
    /// mentions.
    ///
    /// Reported rather than quietly dropped. Claude Code prunes its own old
    /// sessions and plenty of this history predates the corpus entirely, so an
    /// unattributed commit is the ordinary case for anything old — and a reader
    /// comparing these line counts against `git log` needs to know how much of
    /// the history they cover before concluding somebody did less than they did.
    #[serde(default)]
    pub commits: usize,
    #[serde(default)]
    pub unattributed: usize,
    /// The timeline, mined in the same pass and written to its own file.
    ///
    /// **Never serialised with the roster.** It is a hundred times the size and
    /// answers a different question; `/api/agents` must not carry it, and the
    /// miner takes it out and saves it separately.
    #[serde(skip)]
    pub doing: reader::doing::Doing,
    /// Where each renamed file ended up, old name to current.
    ///
    /// Kept in the artefact rather than applied and forgotten, for two reasons:
    /// a query for the name a file *used* to have must still find it, and a
    /// reader looking at forty changes to a file created last week deserves to
    /// be told the history came from somewhere else.
    #[serde(default)]
    pub renames: BTreeMap<String, String>,
    /// Named sessions, busiest first.
    pub agents: Vec<Agent>,
}

impl Agents {
    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// The agent a session id belongs to, for resolving a memory's
    /// `originSessionId` to a name.
    ///
    /// `None` is an ordinary answer, not a failure: Claude Code prunes its own
    /// old sessions, so a memory can outlive the transcript that wrote it —
    /// 24 of the live corpus's memories name a session with no transcript left.
    /// Those keep their raw id rather than being dropped or attributed to
    /// somebody else.
    /// Who has been working on the files a query names, busiest first.
    ///
    /// Substring, case-insensitive, over the whole repo-relative path — so
    /// `dhall` finds both the `kubes/dhall/` directory and every `*.dhall` file,
    /// which are the same question asked two ways and would need two rules to
    /// tell apart for no gain.
    ///
    /// **Ranked by writes, not by total.** The question is who *makes changes of
    /// that sort*; reading widely is a different thing and is reported beside it
    /// rather than folded in. Agents matching nothing are dropped entirely — a
    /// row of zeroes is noise that grows with the roster.
    ///
    /// The matching paths come back with the counts, because a bare ranking is
    /// unfalsifiable: the evidence is what lets a reader see that "dhall" caught
    /// a `.dhall` file and not a directory called `dhallium`.
    pub fn who_works_on(&self, query: &str) -> Vec<WorkMatch> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        // A file keeps the evidence filed under the names it used to have, so a
        // query for one of those names must find it — otherwise renaming a file
        // hides its own history from the only person looking for it.
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
                // The two dimensions are unioned here rather than at mining
                // time, so a file used both ways is one row and not two.
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
                // Committed lines are the same work measured a second way, so
                // they are attached to the row and never added to its counts. A
                // file can appear here having never been opened by a tool call
                // at all — created by a script, or edited on another machine.
                for (path, delta) in agent.commit_lines.iter().filter(|(path, _)| named(path)) {
                    let file = merged.entry(path).or_insert_with(|| blank(path));
                    file.added = delta.added;
                    file.deleted = delta.deleted;
                    file.commits = delta.commits;
                }
                let mut files: Vec<WorkFile> = merged.into_values().collect();
                // Work on other machines, each row saying which one. A remote
                // path is a different path — `/etc/nixos/flake.nix` on odin is
                // not a file here — so it gets its own row rather than being
                // merged into a local one that happens to share a name.
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

/// The tools whose `file_path` counts as reading.
const READ_TOOLS: [&str; 1] = ["Read"];
/// ...and as writing. `NotebookEdit` carries `notebook_path`, not `file_path`,
/// so it is not counted here rather than counted wrongly.
const WRITE_TOOLS: [&str; 2] = ["Write", "Edit"];

/// How long it takes for a day's presence to count half as much.
///
/// Fourteen days is deliberately gentle. The measured alternative was decaying
/// individual file operations, and both shapes were tried against the live
/// corpus: day-presence put more agents on their own project than event decay
/// did, and — unlike event decay — the answer did not move when the half-life
/// was halved. A signal that is insensitive to a tuning constant is one the
/// constant is not secretly carrying.
const HALF_LIFE_DAYS: f64 = 14.0;

/// Days since the epoch for an ISO-8601 stamp, from its `YYYY-MM-DD` prefix.
///
/// Hinnant's civil-days algorithm, inline rather than pulled from a date crate:
/// the whole need is "how many days between these two dates", and the miner
/// otherwise has no date dependency at all.
fn day_number(stamp: &str) -> Option<i64> {
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

/// Weight a set of active days against `today`, newest counting most.
///
/// **Days present, not files touched.** A session that spent one afternoon
/// making seventy-five edits in a repository it has not opened since is not a
/// session that works there, but counting files says it is — and on the live
/// data that single burst outvoted a fortnight of steady work in the project
/// the session is actually named for. Counting the days it showed up cannot be
/// dominated that way: a busy afternoon is worth one day, the same as a quiet
/// one.
///
/// Nothing decays to zero, so an old project fades out of the ordering rather
/// than disappearing — the lifetime counts alongside it stay undecayed.
pub fn recency(days: &std::collections::BTreeSet<i64>, today: i64) -> f64 {
    days.iter()
        .map(|&d| 0.5f64.powf(((today - d).max(0)) as f64 / HALF_LIFE_DAYS))
        .sum()
}

/// The days an agent was present in each project, kept apart from the counts
/// because a day is not a tally — the same day seen twice is still one day.
#[derive(Default)]
struct DaysSeen {
    reads: BTreeMap<String, std::collections::BTreeSet<i64>>,
    writes: BTreeMap<String, std::collections::BTreeSet<i64>>,
}

/// The project a path belongs to: the first element under the code root.
///
/// `None` for anywhere else, which deliberately drops the two largest sources of
/// noise — the scratchpad under `/private/tmp`, where every session writes
/// throwaway scripts, and the memory corpus itself, which every session reads
/// and which says nothing about what any of them works on. The corpus is
/// counted separately, by [`memory_of`], because *which* memory an agent opens
/// says a great deal even though *that* it opens memories says nothing.
fn project_of(path: &str, code_root: &str) -> Option<String> {
    let root = code_root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    let name = rest.split('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// A path's position under the code root — `xinutec-infra/plan/backup.dhall`.
///
/// The project prefix is kept rather than stripped, so a result reads as itself
/// with no second lookup, and so a query naming a repository works like any other
/// substring.
fn relative_to(path: &str, code_root: &str) -> Option<String> {
    let root = code_root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    (!rest.is_empty() && rest.contains('/')).then(|| rest.to_string())
}

/// Whether a path is one that work can be attributed to at all.
///
/// Build output, dependency trees, logs and editor leftovers are files an agent
/// touches *because* of the work rather than *as* the work — `rm -rf dist`
/// changes forty files and says nothing about who owns the code that built
/// them.
///
/// Measured before it was written, on the live corpus: generated paths are 0.1%
/// of tool-call use and **4.3% of shell use**, because `Write` and `Edit`
/// hardly ever address a build directory and `rm`, `>` and `cp` constantly do.
/// The same rule applies to both dimensions even so — one definition of a file
/// worth attributing, not one per source of evidence — and on the tool side it
/// removes 44 uses out of 49,699.
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

/// Whether a path on another machine is one work can be attributed to.
///
/// The same idea as [`attributable`] and a different list, because a remote path
/// is absolute and answers to no code root. Scratch and kernel filesystems go:
/// `/tmp` is where every session drops a throwaway, and reading `/proc` is not
/// work on a file. Logs go for the same reason they do locally — the busiest
/// remote path in the corpus is a drill run's log at 90 reads and 17 writes, and
/// none of that is authorship.
fn remotely_attributable(path: &str) -> bool {
    const SCRATCH: [&str; 5] = ["/tmp/", "/var/tmp/", "/proc/", "/sys/", "/dev/"];
    !SCRATCH.iter().any(|dir| path.starts_with(dir)) && attributable(path)
}

/// The memory a path names, for paths inside the corpus directory.
///
/// The canonical id is the filename stem, matching the rest of the app — the
/// frontmatter `name` is not trusted anywhere else either. Anything that is not
/// a `.md` file directly in the corpus is `None`, so `MEMORY.md` (the index,
/// which every session is given and which distinguishes nobody) is excluded by
/// name.
fn memory_of(path: &str, memory_root: &str) -> Option<String> {
    let root = memory_root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    let stem = rest.strip_suffix(".md")?;
    (!stem.is_empty() && !stem.contains('/') && stem != "MEMORY").then(|| stem.to_string())
}

/// One transcript file and the session whose work it records.
struct Transcript {
    path: std::path::PathBuf,
    /// The session id that owns this work — for a delegated transcript, the
    /// session that dispatched it, not the subagent's own id.
    owner: String,
    delegated: bool,
}

/// Every transcript under a project directory, attributed to its owner.
///
/// The layout is `<project>/<session>.jsonl` for a session's own turns, and
/// `<project>/<session>/subagents/…` — nested again under `workflows/<run>/`
/// for workflow agents — for everything it dispatched.
///
/// **Delegated work belongs to the session that dispatched it.** A subagent has
/// no name, no continuity and no purpose of its own; it exists because a named
/// session asked for it, and its edits are that session's edits. Filing them
/// separately would invent hundreds of one-shot agents and subtract their work
/// from the sessions actually responsible for it.
///
/// It is not a rounding error. On the live corpus about a tenth of all
/// Read/Write/Edit calls happen in delegated transcripts, and the share runs
/// from none at all to a seventh depending on the session — so ignoring them
/// does not merely undercount, it undercounts unevenly, which is what makes
/// agents incomparable rather than uniformly understated.
fn transcripts_under(projects_root: &Path) -> Vec<Transcript> {
    fn descend(dir: &Path, owner: &str, out: &mut Vec<Transcript>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `file_type` comes from the directory entry and does NOT follow
            // symlinks, where `is_dir` would: a link back to an ancestor would
            // otherwise recurse until the stack gives out. It also saves a stat
            // per entry, and there are a thousand of them.
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
    // resolved from the transcript that carries the naming reminder before a
    // subagent — which carries none — can settle the agent under a bare id.
    out.sort_by(|a, b| {
        a.owner
            .cmp(&b.owner)
            .then(a.delegated.cmp(&b.delegated))
            .then(a.path.cmp(&b.path))
    });
    out
}

/// Session id → name, from the live registry at `~/.claude/sessions`.
///
/// Keyed by pid, and each entry carries `sessionId` and `name`. This is the
/// authority over the in-transcript "the user named this session" reminder,
/// which is written once and goes stale the moment a session is renamed.
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
/// forgotten. Stale after a rename, which is why it is only the fallback.
///
/// **The quotes are backslash-escaped**, because the reminder is prose inside a
/// JSON string: the bytes on disk read `named this session \"home\"`. Matching
/// the unescaped form finds nothing at all, and the failure is silent — every
/// session the registry has forgotten simply shows as a bare uuid, which looks
/// like an unnamed session rather than a broken parser.
///
/// First occurrence wins, which is right for the session's own reminder (it is
/// injected near the top) but is not airtight: a transcript can quote another
/// session's name later on, and this one does. Acceptable only because the
/// registry is the authority and this runs when the registry has nothing.
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

/// The name a session is going by *now*, from the line the CLI re-appends as it
/// goes along.
///
/// ⚠ **This, not the registry, is where a chosen name lives.** `~/.claude/sessions`
/// used to hold it and no longer does: every entry there now carries a name the
/// CLI made up for itself — `code-c4`, `code-fa`, the working directory's last
/// segment and two hex digits — so a conversation called `health` was shown as
/// `code-c4` while the console's own front page, which reads these lines, called
/// it `health`. Measured 2026-08-06: all fourteen registry entries were of that
/// form and not one carried a name anybody had chosen.
///
/// **Last occurrence wins**, which is what makes this current where the
/// once-written `named this session` reminder goes stale — a rename appends
/// another of these, and the newest is the answer.
///
/// Anchored on the whole opening of the line, `{"type":"agent-name",`, rather
/// than on the key alone. Inside a transcript every quote of a quoted line is
/// backslash-escaped, so this shape occurs only where the CLI wrote the line
/// itself and never in a tool result that happens to print one — which
/// transcripts on this machine do.
///
/// The session id on the line has to be the transcript's own, so a line copied
/// from elsewhere cannot rename an agent. Returns `None` when the CLI changes the
/// shape, which costs the fallbacks and not correctness.
fn titled_in_transcript(text: &[u8], owner: &str) -> Option<String> {
    // The name it was given, then the title it shows under — the same word in
    // every transcript here, and this order because one is a name and the other
    // is a caption.
    //
    // ⚠ **The console applies the opposite order**, and its reasons are as good
    // as these. Neither is wrong today, because enrolment writes both values the
    // same; `reader::transcript` states the divergence in full and says why it
    // was not settled by a refactor. The spelling of the lines comes from there
    // so that at least the vocabulary cannot drift.
    let written = [
        reader::transcript::name_needle(&reader::transcript::AGENT_NAME),
        reader::transcript::name_needle(&reader::transcript::CUSTOM_TITLE),
    ];
    written
        .iter()
        .find_map(|needle| last_titled(text, needle, owner))
}

/// The value on the last line opening with `needle`, when that line is `owner`'s.
fn last_titled(text: &[u8], needle: &[u8], owner: &str) -> Option<String> {
    let start = last_at(text, needle)? + needle.len();
    let end = find_at(text, b"\"", start)?;
    let line = find_at(text, b"\n", end).unwrap_or(text.len());
    // The id is on the same line, after the name. A line naming another session
    // is not this session's name, however it got here — so no id, no name.
    find_at(&text[end..line], owner.as_bytes(), 0)?;
    let name = std::str::from_utf8(&text[start..end]).ok()?;
    (!name.is_empty() && name.len() <= 40).then(|| name.to_string())
}

/// The key holding the path, inside a tool call's `input` object.
const PATH_KEY: &[u8] = b"\"file_path\":\"";

/// The `Bash` commands on one transcript line, and the directory they ran in.
///
/// Parsed as JSON rather than scanned for a needle, because a command is a JSON
/// string full of escapes — `\"`, `\n`, `\\` — and reading the raw bytes would
/// hand the shell parser text nobody typed. The cheap byte test comes first: the
/// corpus is gigabytes and about one line in forty carries a `Bash` call, so
/// parsing every line would cost minutes to no purpose.
///
/// The `cwd` is the line's own, and is what a relative path in the command is
/// resolved against. `None` where the transcript does not record one, which
/// makes relative paths unusable rather than guessed at.
pub fn bash_calls(line: &[u8]) -> Option<(Option<String>, Vec<String>)> {
    let line = bash_calls_with_ids(line)?;
    Some((
        line.cwd,
        line.calls.into_iter().map(|call| call.command).collect(),
    ))
}

/// One `Bash` call: the command, and the id its result will name.
#[derive(Debug, Clone)]
pub struct BashCall {
    pub id: String,
    pub command: String,
}

/// The `Bash` calls on one transcript line, with the directory they ran in.
#[derive(Debug, Clone)]
pub struct BashLine {
    pub cwd: Option<String>,
    pub calls: Vec<BashCall>,
}

/// As [`bash_calls`], keeping each call's id.
///
/// The id is the join to the result that came back, which arrives on a later
/// line as a `tool_result` naming it — the only way to know whether the work
/// succeeded.
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
            })
        })
        .collect();
    Some(BashLine {
        cwd,
        calls: commands,
    })
}

/// The harness's own words for a call the user would not allow. Anchored to the
/// front of the content, which is what makes it safe to match — see
/// [`reader::doing::Verdict`].
const REFUSED: &[u8] = b"\"content\":\"The user doesn't want to proceed with this tool use";

/// What became of every call in one transcript, by id.
///
/// A pass of its own, because a result is written below the call it answers and
/// a transcript is read from the top.
///
/// ⚠ **`Ok` is kept rather than left implicit.** Absent from this map means *no
/// result came back at all* — interrupted, or still running — which is
/// [`reader::doing::Verdict::Unknown`] and admits nothing. Dropping the
/// successes to save space would make silence and success the same answer.
/// The map is per transcript and freed with it, so there is no space to save.
fn outcomes(text: &[u8]) -> std::collections::HashMap<String, reader::doing::Verdict> {
    let mut out = std::collections::HashMap::new();
    for line in text.split(|c| *c == b'\n') {
        if let Some((call, verdict)) = tool_result(line) {
            out.insert(call, verdict);
        }
    }
    out
}

/// Whether this transcript exists only to name another conversation.
///
/// ⚠ **These are not agents, and they outnumber the agents four to one.** The
/// CLI titles a conversation by handing a summary of it to a one-shot Haiku
/// session, which persists a transcript like any other — 307 of them against 13
/// real sessions, each a bare uuid with every counter at zero, filling the page
/// they were supposed to be measured on.
///
/// **Both halves of the test are needed.** An `ai-title` line alone is not the
/// marker: older CLI versions wrote the title into the working session's *own*
/// transcript, and nine such sessions in the corpus carry one — 11,000 to
/// 110,000 lines, thousands of tool calls, Opus and Fable. Excluding on the line
/// alone would delete the largest sessions there are. A titler makes no tool
/// call at all, and that is what separates them.
///
/// Two markers, because neither alone is complete: four of these produced a
/// title in their reply without an `ai-title` line ever being written, and are
/// recognisable only by the prompt they were handed.
///
/// ⚠ **The tool-call test runs first, and that ordering is load-bearing.** Any
/// session that *investigates* this problem quotes the titling prompt into its
/// own transcript — this very rule was written in one that now contains the
/// string twice. Matching the prompt first would make a session disappear for
/// having looked into why sessions disappear. Making a tool call is what no
/// titler ever does, and it is checked before any text is.
///
/// Deleting the rest is not the alternative: a transcript with no tool call
/// contributes nothing either way, so if these markers ever go stale the cost is
/// a few visible empty rows rather than lost work. A bare "made no tool call"
/// test would be simpler and would also drop the 7 dispatched subagents (of 940)
/// that answered without using one, which is real delegation and worth counting.
fn titling(text: &[u8]) -> bool {
    if find_at(text, b"\"type\":\"tool_use\"", 0).is_some() {
        return false;
    }
    const TITLED: &[u8] = b"\"type\":\"ai-title\"";
    const ASKED: &[u8] = b"Below is part of a conversation between a person and a coding agent";
    find_at(text, TITLED, 0).is_some() || find_at(text, ASKED, 0).is_some()
}

/// Whether the tool call whose name begins at `at` did what it was asked.
///
/// The id belongs to the same JSON object and is written before the name, so the
/// nearest `"id":"` behind the needle is this call's. A line can carry several
/// calls, which is why it is the *nearest* rather than the first.
///
/// A call whose id cannot be read is treated as completed: the alternative is to
/// drop real work over a parse this function is not confident about, and being
/// unable to find an id says nothing about whether the tool acted.
fn call_completed(
    line: &[u8],
    at: usize,
    outcomes: &std::collections::HashMap<String, reader::doing::Verdict>,
) -> bool {
    const ID: &[u8] = b"\"id\":\"";
    let Some(start) = crate::couse::last_at(&line[..at], ID).map(|pos| pos + ID.len()) else {
        return true;
    };
    let Some(end) = find_at(line, b"\"", start) else {
        return true;
    };
    let Ok(id) = std::str::from_utf8(&line[start..end]) else {
        return true;
    };
    outcomes
        .get(id)
        .copied()
        .unwrap_or(reader::doing::Verdict::Unknown)
        .completed()
}

/// The call a `tool_result` line answers, and what became of it.
///
/// Read with needles rather than parsed: a result carries the command's whole
/// output, which is most of the corpus's bytes, and none of that text is wanted
/// — only the id it names and how it went.
///
/// The refusal needle is anchored to the front of the content on purpose; see
/// [`reader::doing::Verdict`] for what goes wrong unanchored.
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

/// The first time each commit hash was mentioned, and by whom.
///
/// A hash does not exist until the commit is made, so the earliest mention is
/// the session that made it. Keyed by full sha; the value is the timestamp and
/// the agent name.
type FirstSeen = BTreeMap<String, (String, String)>;

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
        // The whole candidate must prefix the hash, not just its first seven
        // characters — a longer mention is a stronger claim, and checking it is
        // what keeps a `git log` printing nine from matching the wrong commit.
        for sha in shas.iter().filter(|sha| sha.starts_with(candidate)) {
            match first.get(sha) {
                Some((seen, _)) if seen.as_str() <= stamp => {}
                _ => {
                    first.insert(sha.clone(), (stamp.to_string(), name.to_string()));
                }
            }
        }
    }
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
) {
    // Borrowed field by field: one tool call updates either a project counter
    // or a memory counter, and the compiler cannot see they are disjoint
    // through `agent`.
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
    // Built once per transcript rather than once per line — the needles are
    // fixed and the corpus is millions of lines.
    let needles: Vec<(String, bool)> = READ_TOOLS
        .iter()
        .map(|tool| (format!("\"name\":\"{tool}\",\"input\":{{"), false))
        .chain(
            WRITE_TOOLS
                .iter()
                .map(|tool| (format!("\"name\":\"{tool}\",\"input\":{{"), true)),
        )
        .collect();
    for line in text.split(|&c| c == b'\n') {
        if line.is_empty() {
            continue;
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
        // Which commits this session knew about, and when. Attribution happens
        // after every transcript has been read, because "first" is a claim about
        // all of them and cannot be settled one at a time.
        if !index.is_empty() {
            note_hashes(line, stamp, index, agent_name, first);
        }
        // What the shell did, beside what the tools did. Counted into its own
        // map and into no project or recency counter: this is a new dimension,
        // not a correction to the old ones.
        //
        // A command the grammar cannot read contributes nothing and is not
        // reported here — `shell-report` is where that is measured, against a
        // corpus, rather than buried in a mine that takes minutes to run.
        // The result of a call made earlier in this transcript, which is how a
        // row learns whether the work succeeded.
        if let Some((call, verdict)) = tool_result(line) {
            log.resolve(&call, verdict);
        }
        if let Some(BashLine { cwd, calls }) = bash_calls_with_ids(line) {
            for BashCall { id: call, command } in calls {
                let Ok(parsed) = reader::shell::parse(&command) else {
                    continue;
                };
                let found = reader::shell_files::extract(&parsed, cwd.as_deref(), home);
                // ⚠ **What the text required, met with what the call returned.**
                // The timeline row goes in whatever happened — being refused or
                // failing is part of the record — but a path only reaches
                // somebody's name when the command that opened it certainly
                // ran. Absent from the map is a call that never answered.
                let verdict = outcomes
                    .get(&call)
                    .copied()
                    .unwrap_or(reader::doing::Verdict::Unknown);
                // What this turn was doing, one row per kind of work in it.
                // Grouped rather than one row per command: a call that runs
                // `sed` over four files is one edit to anybody reading it.
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
                // A refusal drops the whole call; everything else is recorded
                // under one of two claims, never thrown away for being unsure.
                for used in found
                    .files
                    .into_iter()
                    .filter(|_| verdict != reader::doing::Verdict::Rejected)
                {
                    let Some(rel) = relative_to(&used.path, code_root).filter(|p| attributable(p))
                    else {
                        continue;
                    };
                    let certain = verdict.admits(used.reached);
                    // **The day counts, even though the count does not.**
                    // Recency decides which project an agent is listed under,
                    // and deciding that from tool calls alone was the very
                    // unevenness this dimension exists to correct: a session
                    // that does its editing through `sed` or a `python3 -`
                    // heredoc was present in that repository, and ordering it
                    // below a session that opened one file with `Edit` says
                    // otherwise. The displayed totals stay apart; only the
                    // ordering signal is made whole.
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
                // Work on another machine, kept under its host. No code-root
                // filter: `/etc/nixos` is where odin's work lives and there is
                // no `~/Code` there — the shape of the filesystem is the remote
                // machine's business, not this one's.
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
        // A line can carry more than one tool call, so every occurrence is
        // walked rather than only the first — a batched turn that opens six
        // files is six reads, and counting it as one would understate exactly
        // the sessions that work hardest.
        for (head, is_write) in &needles {
            let is_write = *is_write;
            let (counter, days) = if is_write {
                (&mut *agent_writes, &mut seen.writes)
            } else {
                (&mut *agent_reads, &mut seen.reads)
            };
            let mut from = 0;
            while let Some(at) = find_at(line, head.as_bytes(), from) {
                let input = at + head.len();
                from = input;
                // ⚠ **A tool call that failed did nothing.** `Edit` fails when
                // its `old_string` is absent and leaves the file untouched; 990
                // of them in the corpus, plus 289 `Write`s, were counted as
                // changes to files they never altered. One thing, one result —
                // none of the reachability reasoning a shell script needs.
                //
                // The id sits just before the name in the same object, so the
                // nearest one behind this needle is this call's.
                if !call_completed(line, at, &outcomes) {
                    continue;
                }
                // **`file_path` is not always the input's first key.** `Edit`
                // serialises `replace_all` ahead of it — every one of the 28,546
                // in the live corpus — so a needle demanding the path directly
                // after the tool name matched none of them at all, and the miner
                // reported zero edits while calling the number "writes". The
                // path is looked up inside the object instead.
                //
                // Bounded by the next tool call so one carrying no path cannot
                // borrow the following call's. A tool's own payload cannot forge
                // either marker: it is a JSON string, so its quotes arrive
                // backslash-escaped and match neither needle.
                let limit = find_at(line, b"\"name\":\"", input).unwrap_or(line.len());
                let Some(key) = find_at(line, PATH_KEY, input) else {
                    break;
                };
                if key >= limit {
                    continue;
                }
                let start = key + PATH_KEY.len();
                let Some(end) = find_at(line, b"\"", start) else {
                    break;
                };
                if let Ok(path) = std::str::from_utf8(&line[start..end]) {
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
                        let use_ = memories.entry(memory).or_default();
                        if is_write {
                            use_.edits += 1;
                        } else {
                            use_.reads += 1;
                        }
                    }
                }
                from = end;
            }
        }
    }
}

/// Mine every transcript under `projects_root` into per-agent directory counts.
///
/// Every project directory is walked, not just one: agents are named per
/// session and a session's transcripts live under whichever root it was started
/// in, so scoping to one root would silently lose whole agents. Work a session
/// delegated counts as its own — see [`transcripts_under`].
///
/// `memory_root` is the corpus directory, so opening a memory is attributed to
/// the memory rather than discarded as "outside the code root". A path that
/// does not exist is harmless: nothing matches it and the profile is empty.
pub fn scan(
    projects_root: &Path,
    sessions_dir: &Path,
    code_root: &str,
    memory_root: &str,
    home: &str,
    generated: &str,
) -> Result<Agents> {
    let names = registry_names(sessions_dir);
    let mut by_name: BTreeMap<String, Agent> = BTreeMap::new();
    // The timeline, built across every transcript and frozen at the end.
    let mut log = reader::doing::Log::default();
    let mut days: BTreeMap<String, DaysSeen> = BTreeMap::new();
    // Read before the transcripts, because recognising a hash in one needs the
    // set of hashes to look for. Empty when the code root has no repositories,
    // in which case the whole dimension is skipped rather than half-built.
    let history = crate::commits::all(Path::new(code_root));
    let mut index: ShaIndex = BTreeMap::new();
    for commit in &history {
        if commit.sha.len() >= crate::commits::SHORT {
            index
                .entry(commit.sha[..crate::commits::SHORT].to_string())
                .or_default()
                .push(commit.sha.clone());
        }
    }
    let mut first_seen: FirstSeen = BTreeMap::new();
    // "Now" is the mine's own stamp, not the wall clock, so the weights are a
    // property of the artefact and re-reading it never changes what it says.
    let today = day_number(generated).unwrap_or(0);

    std::fs::metadata(projects_root)
        .with_context(|| format!("reading {}", projects_root.display()))?;
    // The name an owner settled on, so a dispatched transcript lands under the
    // same agent as the session that dispatched it.
    let mut resolved: BTreeMap<String, String> = BTreeMap::new();

    for transcript in transcripts_under(projects_root) {
        let Ok(text) = std::fs::read(&transcript.path) else {
            continue;
        };
        if titling(&text) {
            continue;
        }
        // The name it goes by now, then the registry, then the reminder it was
        // given once, then the id.
        //
        // ⚠ **The registry used to come first and no longer can.** It is still
        // live where the once-written reminder goes stale, which was the whole
        // argument for it — but what it now holds is a name the CLI made up
        // (`code-c4`), so trusting it first renamed every conversation on the
        // page to a placeholder. See [`titled_in_transcript`]. It stays ahead of
        // the reminder, and a session nobody named still shows the CLI's short
        // handle in preference to a bare uuid.
        //
        // An unnamed session is shown as its id rather than merged into an
        // "unknown" bucket — several distinct agents pooled under one label
        // would be a claim about the work that nothing supports.
        let name = resolved
            .entry(transcript.owner.clone())
            .or_insert_with(|| {
                // Only a session's own transcript names it; a subagent carries
                // its parent's context, and quoting a name is not being called
                // one.
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
        // The owner id, so a delegated transcript records the session that
        // dispatched it rather than the subagent's own id — the same identity
        // the rest of the row is counted under, and the one a memory's
        // `originSessionId` will name.
        agent.sessions.insert(transcript.owner.clone());
        if transcript.delegated {
            agent.delegated += 1;
        } else {
            agent.transcripts += 1;
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
        );
    }

    for (name, seen) in &days {
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

    // Every transcript has now been read, so "who saw this hash first" is a
    // question with an answer. A commit nobody mentioned goes to nobody: the
    // transcripts are pruned by Claude Code, and work predating the corpus has
    // no session left to credit.
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

    // A file that was renamed has been filed under both of its names all along
    // — by the tool calls that edited it before the move, by the shell that
    // touched it after, and by git on either side. Git is the only one of the
    // three that knows the two are one file, so its answer is applied to all of
    // them here, at the end, where every dimension is complete.
    let renames = renames(&history);
    let mut agents: Vec<Agent> = by_name.into_values().collect();
    for agent in &mut agents {
        agent.paths = rename_keys(std::mem::take(&mut agent.paths), &renames);
        agent.shell_paths = rename_keys(std::mem::take(&mut agent.shell_paths), &renames);
        agent.commit_lines = rename_keys(std::mem::take(&mut agent.commit_lines), &renames);
    }
    agents.sort_by_key(|a| {
        let total: usize = a.reads.values().sum::<usize>() + a.writes.values().sum::<usize>();
        (std::cmp::Reverse(total), a.name.clone())
    });
    Ok(Agents {
        doing: log.finish(generated),
        generated: generated.to_string(),
        commits: history.len(),
        unattributed,
        renames,
        agents,
    })
}

/// Where each old path ended up, following a chain of renames to the end.
///
/// The history arrives newest-first, so a file renamed twice is met at its
/// latest name first and the chain is walked forward from each entry rather
/// than assumed to be one step. Bounded, because a rename cycle — `a → b` in
/// one commit and `b → a` in another — is a thing git will happily record.
fn renames(history: &[crate::commits::Commit]) -> BTreeMap<String, String> {
    let mut step: BTreeMap<String, String> = BTreeMap::new();
    for commit in history {
        for file in &commit.files {
            if let Some(was) = &file.was
                && was != &file.path
            {
                // Newest first, so an earlier commit's rename of the same name
                // is the *older* fact and must not overwrite the newer one.
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

/// Adding one file's figures to another's — what re-keying two names onto one
/// file has to do with the two sets of counts it finds there.
pub trait Merge {
    fn merge(&mut self, other: Self);
}

impl Merge for MemoryUse {
    fn merge(&mut self, other: Self) {
        // ⚠ **Destructured so a new field cannot quietly go unmerged.**
        // `rename_keys` rebuilds every path map through this, so a counter
        // missing here is not merely un-added — it is reset to zero for every
        // entry, renamed or not. Naming the fields makes that a compile error
        // instead of a silent loss, which is how `maybe_reads` was first lost.
        let MemoryUse {
            reads,
            edits,
            maybe_reads,
            maybe_edits,
        } = other;
        self.reads += reads;
        self.edits += edits;
        self.maybe_reads += maybe_reads;
        self.maybe_edits += maybe_edits;
    }
}

impl Merge for LineDelta {
    fn merge(&mut self, other: Self) {
        self.added += other.added;
        self.deleted += other.deleted;
        self.commits += other.commits;
    }
}

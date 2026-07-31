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
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::couse::{field, find_at};

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
    /// Files opened, per project directory. Lifetime totals, undecayed — the
    /// honest record of what happened, and what the totals line reports.
    pub reads: BTreeMap<String, usize>,
    /// Files written or edited, per project directory. Lifetime, undecayed.
    pub writes: BTreeMap<String, usize>,
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
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Agents {
    /// When this was mined, ISO-8601 — the artefact's own account of its age,
    /// because an mtime records the last copy rather than the last derivation.
    #[serde(default)]
    pub generated: String,
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
            } else if path.extension().is_some_and(|e| e == "jsonl") {
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
            } else if path.extension().is_some_and(|e| e == "jsonl") {
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

/// The key holding the path, inside a tool call's `input` object.
const PATH_KEY: &[u8] = b"\"file_path\":\"";

/// Count one transcript's tool calls into `agent`, and note the days.
fn scan_transcript(
    text: &[u8],
    code_root: &str,
    memory_root: &str,
    agent: &mut Agent,
    seen: &mut DaysSeen,
) {
    // Borrowed field by field: one tool call updates either a project counter
    // or a memory counter, and the compiler cannot see they are disjoint
    // through `agent`.
    let Agent {
        reads: agent_reads,
        writes: agent_writes,
        memories,
        first,
        last,
        ..
    } = agent;
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
        if let Some(stamp) = field(line, "timestamp").and_then(|t| std::str::from_utf8(t).ok()) {
            if first.is_empty() || stamp < first.as_str() {
                *first = stamp.to_string();
            }
            if stamp > last.as_str() {
                *last = stamp.to_string();
            }
            day = day_number(stamp);
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
    generated: &str,
) -> Result<Agents> {
    let names = registry_names(sessions_dir);
    let mut by_name: BTreeMap<String, Agent> = BTreeMap::new();
    let mut days: BTreeMap<String, DaysSeen> = BTreeMap::new();
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
        // Registry first, transcript second, id last. An unnamed session is
        // shown as its id rather than merged into an "unknown" bucket —
        // several distinct agents pooled under one label would be a claim
        // about the work that nothing supports.
        let name = resolved
            .entry(transcript.owner.clone())
            .or_insert_with(|| {
                names
                    .get(&transcript.owner)
                    .cloned()
                    // Only a session's own transcript carries the naming
                    // reminder; a subagent that quotes one is quoting its
                    // parent's context, not naming itself.
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
        if transcript.delegated {
            agent.delegated += 1;
        } else {
            agent.transcripts += 1;
        }
        scan_transcript(
            &text,
            code_root,
            memory_root,
            agent,
            days.entry(agent.name.clone()).or_default(),
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

    let mut agents: Vec<Agent> = by_name.into_values().collect();
    agents.sort_by_key(|a| {
        let total: usize = a.reads.values().sum::<usize>() + a.writes.values().sum::<usize>();
        (std::cmp::Reverse(total), a.name.clone())
    });
    Ok(Agents {
        generated: generated.to_string(),
        agents,
    })
}

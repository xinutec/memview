//! Which memories get used together in one unit of thinking — how the corpus
//! is USED, where the written `[[links]]` say how it describes itself. Mined
//! from the session transcripts.
//!
//! The unit is a turn (`promptId`), not a session and not a clock window:
//! sessions span weeks of unrelated work, and several run at once. A session is
//! a tree — `parentUuid` chains messages, a rewind starts a sibling branch — so
//! `promptId`, present on a third of messages, is inherited down the parent
//! chain rather than carried in file order.
//!
//! Nothing but names and counts leaves this module. The transcripts contain the
//! whole medical case file and every credential ever pasted; the artefact is
//! memory names, project names and integers. An earlier version served the
//! literal history and was removed for exactly that.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Two memories, and the evidence that they belong together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pair {
    pub a: String,
    pub b: String,
    /// Separate turns in which both were used — evidence, not the statistic.
    pub turns: usize,
    /// Distinct sessions those turns came from. This is the support.
    pub sessions: usize,
    /// Normalised pointwise mutual information; the artefact carries only (0, 1]
    /// because the mine drops below-chance pairs — see [`scan`]. Raw counts rank
    /// the hubs and nothing else.
    pub npmi: f64,
}

/// Signs of life for one memory: a rule cited by six projects and never once
/// consulted looks identical, in a link graph, to the one that governs every session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Distinct sessions that mentioned it at all.
    pub sessions: usize,
    /// Turns that mentioned it — finer, and dominated by long pieces of work.
    pub turns: usize,
    /// Times it was deliberately opened with Read.
    pub reads: usize,
    /// Times it was written or edited.
    pub edits: usize,
    /// Most recent mention, ISO-8601, or absent if never seen.
    pub last: Option<String>,
    /// Mentions per project, from the `cwd` of the line that named it — the context
    /// a memory is consulted in, which its text does not say. `cwd` rather than any
    /// text signal: MEMORY.md names every project and is injected everywhere.
    #[serde(default)]
    pub projects: BTreeMap<String, usize>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CoUse {
    /// When this was mined, ISO-8601: the file's mtime is when it was last COPIED.
    #[serde(default)]
    pub generated: String,
    /// Turns that used at least two memories — the denominator.
    pub turns: usize,
    /// Pairs above the session floor, strongest first.
    pub pairs: Vec<Pair>,
    /// Per-memory usage, for every memory seen at least once.
    #[serde(default)]
    pub usage: BTreeMap<String, Usage>,
}

/// Distinct sessions a pair must meet in before it is reported. The session is
/// the sample; the turn is only how a meeting is detected — scored per turn, the
/// strongest pair was 710 turns of one health-sync week. Small numbers: there
/// are only about thirteen sessions.
pub const MIN_SESSIONS: usize = 3;

/// Most memories one turn may contribute before it is discarded: a turn touching
/// half the corpus has quoted MEMORY.md or bulk-edited the directory.
const MAX_BASKET: usize = 40;

/// Most memories one LINE may contribute: a line naming many is a listing — a
/// grep result, or a bullet of MEMORY.md naming seventeen, which manufactured
/// 136 pairs that outranked everything real. The per-turn cap does not catch it.
const MAX_PER_LINE: usize = 6;

impl CoUse {
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        crate::atomic::write(path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Pairs used together that the corpus connects nowhere — neither directly nor
    /// through a memory linking both. Returns the worklist and the count held back.
    /// A shared neighbour is a connection, and counting it removed 59% of the
    /// report: a hub and its own children co-occur BECAUSE the hub sent the reader
    /// to each. One hop only.
    pub fn unlinked<'a>(
        &'a self,
        adjacency: &BTreeMap<String, BTreeSet<String>>,
    ) -> (Vec<&'a Pair>, usize) {
        let empty = BTreeSet::new();
        let neighbours = |n: &str| adjacency.get(n).unwrap_or(&empty);
        let mut worklist = Vec::new();
        let mut connected = 0usize;
        for p in &self.pairs {
            let (a, b) = (neighbours(&p.a), neighbours(&p.b));
            if a.contains(&p.b) || b.contains(&p.a) || a.intersection(b).next().is_some() {
                connected += 1;
            } else {
                worklist.push(p);
            }
        }
        (worklist, connected)
    }
}

/// Find `needle` in `hay` starting at `from`.
pub(crate) fn find_at(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Find the LAST `needle` in `hay`, searched from the end: the line the CLI
/// re-appends sits a few kilobytes from the end of a file that can be gigabytes.
pub(crate) fn last_at(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .rev()
        .find(|&i| &hay[i..i + needle.len()] == needle)
}

/// The value of a `"key":"…"` field, as bytes, without parsing the line: a
/// `serde_json` parse of every line costs minutes across three gigabytes.
/// `"uuid":"` cannot collide with `"parentUuid":"`.
pub(crate) fn field<'a>(line: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let needle = format!("\"{key}\":\"");
    let start = find_at(line, needle.as_bytes(), 0)? + needle.len();
    let end = find_at(line, b"\"", start)?;
    Some(&line[start..end])
}

/// Every corpus memory named anywhere in the line — any mention, not only a
/// file opened: counting reads gave 14 usable pairs against 829, since most
/// memories reach a session by recall. Sound because a pair needs two memories
/// in ONE turn; it does not generalise to single memories — see
/// [`crate::agents::MemoryUse`].
fn names_in(line: &[u8], corpus: &BTreeSet<String>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    // Candidate runs of [a-z0-9_], checked against the corpus; cannot invent one.
    let mut start = None;
    for (i, &c) in line.iter().enumerate() {
        let wordish = c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_';
        match (wordish, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s >= 5
                    && let Ok(word) = std::str::from_utf8(&line[s..i])
                    && corpus.contains(word)
                {
                    found.insert(word.to_string());
                }
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// The project a working directory belongs to. `code_root` is a parameter
/// because this is a public repo.
pub fn project_of(cwd: &str, code_root: &str) -> Option<String> {
    let root = code_root.trim_end_matches('/');
    let rest = cwd.strip_prefix(root)?.strip_prefix('/')?;
    let name = rest.split('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Scan one transcript into `(turn key) -> names used`.
fn scan_session(
    path: &Path,
    corpus: &BTreeSet<String>,
    session: usize,
    code_root: &str,
    out: &mut Vec<(usize, BTreeSet<String>)>,
    usage: &mut BTreeMap<String, Usage>,
) -> Result<()> {
    let text = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;

    let mut parent_of: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let mut prompt_of: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    // (uuid, name) pairs, resolved to turns once the whole tree is known.
    let mut refs: Vec<(Vec<u8>, String)> = Vec::new();

    for line in text.split(|&c| c == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Some(uuid) = field(line, "uuid") else {
            continue;
        };
        let uuid = uuid.to_vec();
        if let Some(parent) = field(line, "parentUuid") {
            parent_of.insert(uuid.clone(), parent.to_vec());
        }
        if let Some(prompt) = field(line, "promptId") {
            prompt_of.insert(uuid.clone(), prompt.to_vec());
        }
        let named = names_in(line, corpus);
        if named.len() > MAX_PER_LINE {
            continue;
        }
        // Which tool, if any, this line is a call of. A line can in principle carry
        // more than one; in practice it does not.
        let opened = find_at(line, b"\"name\":\"Read\"", 0).is_some();
        let written = find_at(line, b"\"name\":\"Write\"", 0).is_some()
            || find_at(line, b"\"name\":\"Edit\"", 0).is_some();
        let stamp =
            field(line, "timestamp").and_then(|t| std::str::from_utf8(t).ok().map(str::to_string));
        // Attributed per line, not per turn: a turn can move between directories.
        let project = field(line, "cwd")
            .and_then(|c| std::str::from_utf8(c).ok())
            .and_then(|c| project_of(c, code_root));
        for name in named {
            let entry = usage.entry(name.clone()).or_default();
            entry.turns += 1;
            if opened {
                entry.reads += 1;
            }
            if written {
                entry.edits += 1;
            }
            if let Some(project) = &project {
                *entry.projects.entry(project.clone()).or_insert(0) += 1;
            }
            // The maximum rather than the last seen: filename order is not chronological.
            if let Some(stamp) = &stamp
                && entry
                    .last
                    .as_deref()
                    .is_none_or(|prev| prev < stamp.as_str())
            {
                entry.last = Some(stamp.clone());
            }
            refs.push((uuid.clone(), name));
        }
    }

    // Nearest ancestor-or-self carrying a promptId, memoised.
    let mut turn_cache: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let mut turn_of = |uuid: &[u8]| -> Vec<u8> {
        let mut walked: Vec<Vec<u8>> = Vec::new();
        let mut cur = uuid.to_vec();
        let answer = loop {
            if let Some(hit) = turn_cache.get(&cur) {
                break hit.clone();
            }
            if let Some(p) = prompt_of.get(&cur) {
                break p.clone();
            }
            walked.push(cur.clone());
            match parent_of.get(&cur) {
                // No promptId up the chain: the message predates the field. Its own id
                // becomes the turn, so orphans do not pool into one basket.
                None => break uuid.to_vec(),
                Some(p) => cur = p.clone(),
            }
        };
        for w in walked {
            turn_cache.insert(w, answer.clone());
        }
        answer
    };

    let mut baskets: BTreeMap<Vec<u8>, BTreeSet<String>> = BTreeMap::new();
    for (uuid, name) in refs {
        baskets.entry(turn_of(&uuid)).or_default().insert(name);
    }
    out.extend(
        baskets
            .into_values()
            .filter(|b| b.len() >= 2 && b.len() <= MAX_BASKET)
            .map(|b| (session, b)),
    );
    Ok(())
}

/// Mine every transcript in `dir` for memories used together.
pub fn scan(
    dir: &Path,
    corpus: &BTreeSet<String>,
    code_root: &str,
    generated: &str,
) -> Result<CoUse> {
    let mut baskets: Vec<(usize, BTreeSet<String>)> = Vec::new();
    let mut usage: BTreeMap<String, Usage> = BTreeMap::new();
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading transcripts in {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| reader::transcript::is_transcript(p))
        .collect();
    paths.sort();
    let session_count = paths.len();
    for (i, path) in paths.iter().enumerate() {
        scan_session(path, corpus, i, code_root, &mut baskets, &mut usage)?;
    }

    let total = baskets.len();
    if total == 0 {
        return Ok(CoUse {
            generated: generated.to_string(),
            ..CoUse::default()
        });
    }
    // Sessions per memory: turns inside one session are one piece of work.
    {
        let mut per_name: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
        for (session, basket) in &baskets {
            for name in basket {
                per_name.entry(name).or_default().insert(*session);
            }
        }
        for (name, sessions) in per_name {
            if let Some(u) = usage.get_mut(name) {
                u.sessions = sessions.len();
            }
        }
    }
    // Per-session presence, which is what makes the session the sampling unit.
    let mut name_sessions: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    let mut pair_sessions: BTreeMap<(&str, &str), BTreeSet<usize>> = BTreeMap::new();
    let mut pair_turns: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (session, basket) in &baskets {
        let names: Vec<&str> = basket.iter().map(String::as_str).collect();
        for (i, a) in names.iter().enumerate() {
            name_sessions.entry(a).or_default().insert(*session);
            for b in &names[i + 1..] {
                pair_sessions.entry((a, b)).or_default().insert(*session);
                *pair_turns.entry((a, b)).or_default() += 1;
            }
        }
    }
    let freq: BTreeMap<&str, usize> = name_sessions.iter().map(|(k, v)| (*k, v.len())).collect();
    let together: BTreeMap<(&str, &str), usize> =
        pair_sessions.iter().map(|(k, v)| (*k, v.len())).collect();

    let n = session_count as f64;
    let mut pairs: Vec<Pair> = together
        .into_iter()
        .filter(|(_, k)| *k >= MIN_SESSIONS)
        .map(|((a, b), k)| {
            let (pa, pb, pab) = (freq[a] as f64 / n, freq[b] as f64 / n, k as f64 / n);
            Pair {
                a: a.to_string(),
                b: b.to_string(),
                turns: pair_turns[&(a, b)],
                sessions: k,
                // pab = 1 makes the formula 0/0: a pair present in EVERY session is 1 by
                // definition, not NaN — which would fail the filter below and drop the
                // best-supported pair in a small corpus.
                npmi: if pab >= 1.0 {
                    1.0
                } else {
                    (pab / (pa * pb)).ln() / -pab.ln()
                },
            }
        })
        // Below chance is not an affinity: two popular memories sharing the minimum
        // three sessions can co-occur LESS than chance, and keeping those made 78% of
        // the artefact weightless (memview#1307). Dropped at the mine, where every
        // consumer agrees.
        .filter(|pair| pair.npmi > 0.0)
        .collect();
    pairs.sort_by(|x, y| {
        y.npmi
            .partial_cmp(&x.npmi)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(y.sessions.cmp(&x.sessions))
            .then(x.a.cmp(&y.a))
    });
    Ok(CoUse {
        generated: generated.to_string(),
        turns: total,
        pairs,
        usage,
    })
}

/// ISO-8601 UTC from a unix timestamp, without a date crate for one string.
pub fn stamp(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (mut y, mut d) = (1970i64, days as i64);
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0;
    while d >= months[m] {
        d -= months[m];
        m += 1;
    }
    format!(
        "{y:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        m + 1,
        d + 1,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

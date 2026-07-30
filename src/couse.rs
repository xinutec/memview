//! Which memories get used together in one unit of thinking.
//!
//! The written `[[links]]` say how the corpus *describes* itself. This says how
//! it is actually *used*: two memories that keep turning up in the same piece of
//! work belong near each other whether or not either one ever mentions the
//! other. Mined from the Claude session transcripts, which are the only record
//! of that.
//!
//! **The unit is a turn, not a session, and not a clock window.** Sessions here
//! run to gigabytes and span weeks of unrelated work, so whole-session
//! co-occurrence would relate everything to everything. Wall-clock is worse
//! still: several sessions run at once and are not thinking the same thoughts,
//! so two memories a second apart in different sessions share nothing.
//! `promptId` groups everything done in service of one user request, which is
//! the closest thing in the data to one thought.
//!
//! **A session is a tree, not a line.** `parentUuid` chains each message to its
//! predecessor, and a rewind or an edited prompt starts a sibling branch — two
//! branches are alternative histories that were never in one context together.
//! `promptId` is present on only about a third of messages, so it is inherited
//! *down the parent chain* rather than carried forward in file order, which
//! keeps those branches apart instead of merging them.
//!
//! **Nothing but names and counts leaves this module.** The transcripts contain
//! everything — the whole of the medical case file, every credential ever
//! pasted, every private conversation. The artefact is a list of memory names
//! and integers, and it is worth keeping it that way even though it never
//! leaves the Mac today.

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
    /// Normalised pointwise mutual information, in (-1, 1].
    ///
    /// Raw counts rank the hubs and nothing else — `project_dev_lint` appears
    /// everywhere, so it pairs highly with everything and says nothing. This
    /// measures how much more often two memories appear together than chance
    /// would give, which is what "these belong together" actually means.
    pub npmi: f64,
}

/// Signs of life for one memory: how much it is actually used, and when last.
///
/// The graph could already show how *connected* a memory is — that is just its
/// links. It could not show whether anyone ever goes there. A rule cited by six
/// projects and never once consulted looks identical, in a link graph, to the
/// one that governs every session.
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
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CoUse {
    /// Turns that used at least two memories — the denominator.
    pub turns: usize,
    /// Pairs above the session floor, strongest first.
    pub pairs: Vec<Pair>,
    /// Per-memory usage, for every memory seen at least once.
    #[serde(default)]
    pub usage: BTreeMap<String, Usage>,
}

/// Distinct sessions a pair must meet in before it is reported at all.
///
/// **The session is the sample; the turn is only how a meeting is detected.**
/// Turns inside one session are not independent observations — they are one
/// piece of work counted many times. Scored per turn, the strongest pair in the
/// corpus was 710 turns of a single health-sync week, outranking every genuine
/// relationship while saying no more than "these two came up a lot that
/// Tuesday". Requiring the pair in one turn keeps the precision that made the
/// turn the right detector; counting sessions keeps the statistics honest.
///
/// Small numbers, because there are only thirteen sessions in total. A pair
/// found in three separate ones, days apart, is a relationship the work keeps
/// rediscovering.
pub const MIN_SESSIONS: usize = 3;

/// Most memories one turn may contribute before it is discarded entirely.
///
/// A turn that touches half the corpus is not thinking about half the corpus —
/// it has quoted MEMORY.md, which lists every memory by name in one blob, or it
/// is a bulk edit over the whole directory. Left in, one such turn manufactures
/// tens of thousands of pairs and drowns everything real.
const MAX_BASKET: usize = 40;

/// Most memories one *line* may contribute.
///
/// A line naming many memories is a listing, not a thought: a `ls` of the corpus
/// directory, a grep result, or — the one that actually poisoned this — a line
/// of MEMORY.md. The index groups related memories onto single bullets, one of
/// which names seventeen, and the index is injected into every session. Left in,
/// those seventeen co-occur in almost every turn and manufacture 136 pairs that
/// outrank everything real: the top of the first ranking was six rules that
/// share one bullet in the index and are otherwise unconnected.
///
/// The per-turn cap does not catch it, because seventeen is well under forty.
const MAX_PER_LINE: usize = 6;

impl CoUse {
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Pairs that are used together but that neither memory links, strongest
    /// first — the corpus being told what it is missing.
    pub fn unlinked<'a>(&'a self, linked: &BTreeSet<(String, String)>) -> Vec<&'a Pair> {
        self.pairs
            .iter()
            .filter(|p| {
                let key = if p.a < p.b {
                    (p.a.clone(), p.b.clone())
                } else {
                    (p.b.clone(), p.a.clone())
                };
                !linked.contains(&key)
            })
            .collect()
    }
}

/// Find `needle` in `hay` starting at `from`.
fn find_at(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// The value of a `"key":"…"` field, as bytes, without parsing the line.
///
/// A full `serde_json` parse of every line costs minutes across three gigabytes,
/// and all but a handful of lines mention no memory at all. Note that probing
/// for `"uuid":"` cannot collide with `"parentUuid":"` — the latter capitalises
/// the U, so the lowercase needle does not occur in it.
fn field<'a>(line: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let needle = format!("\"{key}\":\"");
    let start = find_at(line, needle.as_bytes(), 0)? + needle.len();
    let end = find_at(line, b"\"", start)?;
    Some(&line[start..end])
}

/// Every corpus memory named anywhere in the line.
///
/// Deliberately any mention, not only a file that was opened. Counting only
/// deliberate reads was tried first and is far too sparse — 14 usable pairs
/// against 829 — because most memories reach a session by being recalled into
/// context, never by being opened. A name written in prose or in reasoning is
/// the memory having been *thought about*, which is the thing being measured.
fn names_in(line: &[u8], corpus: &BTreeSet<String>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    // Candidate runs of [a-z0-9_], checked against the corpus. Cheaper than
    // searching for each of ~350 names, and it cannot invent one.
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

/// Scan one transcript into `(turn key) -> names used`.
fn scan_session(
    path: &Path,
    corpus: &BTreeSet<String>,
    session: usize,
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
        // Which tool, if any, this line is a call of. A line can in principle
        // carry more than one call; in practice it does not, and attributing a
        // read to the wrong one of two memories named together is a smaller
        // error than not counting reads at all. Stated because it is a limit,
        // not because it is exact.
        let opened = find_at(line, b"\"name\":\"Read\"", 0).is_some();
        let written = find_at(line, b"\"name\":\"Write\"", 0).is_some()
            || find_at(line, b"\"name\":\"Edit\"", 0).is_some();
        let stamp =
            field(line, "timestamp").and_then(|t| std::str::from_utf8(t).ok().map(str::to_string));
        for name in named {
            let entry = usage.entry(name.clone()).or_default();
            entry.turns += 1;
            if opened {
                entry.reads += 1;
            }
            if written {
                entry.edits += 1;
            }
            // Kept as the maximum rather than the last seen: transcripts are
            // scanned in filename order, which is not chronological order.
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

    // Nearest ancestor-or-self carrying a promptId. Memoised, because a deep
    // chain would otherwise be rewalked for every reference on it.
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
                // No promptId anywhere up the chain: the message predates the
                // field. Its own id becomes the turn, which keeps it from
                // pooling with unrelated orphans into one enormous basket.
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
pub fn scan(dir: &Path, corpus: &BTreeSet<String>) -> Result<CoUse> {
    let mut baskets: Vec<(usize, BTreeSet<String>)> = Vec::new();
    let mut usage: BTreeMap<String, Usage> = BTreeMap::new();
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading transcripts in {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    paths.sort();
    let session_count = paths.len();
    for (i, path) in paths.iter().enumerate() {
        scan_session(path, corpus, i, &mut baskets, &mut usage)?;
    }

    let total = baskets.len();
    if total == 0 {
        return Ok(CoUse::default());
    }
    // Sessions per memory, which is the honest "how often is this used" — turns
    // inside one session are one piece of work, so a memory hammered for a week
    // and never touched again would otherwise outrank one consulted every week.
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
    // Per-session presence: which names appeared in any turn, and which pairs
    // met inside some single turn. Collapsing to presence here is what makes the
    // session the sampling unit.
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
                npmi: (pab / (pa * pb)).ln() / -pab.ln(),
            }
        })
        .collect();
    pairs.sort_by(|x, y| {
        y.npmi
            .partial_cmp(&x.npmi)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(y.sessions.cmp(&x.sessions))
            .then(x.a.cmp(&y.a))
    });
    Ok(CoUse {
        turns: total,
        pairs,
        usage,
    })
}

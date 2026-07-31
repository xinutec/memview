//! Ranking for the memory search.
//!
//! The search this replaces was a literal substring match over name, description
//! and body, scored `name*4 + description*2 + body*1`. Both halves of that failed,
//! and the second one failed silently.
//!
//! **Multi-word queries returned nothing at all.** `"launchd TCC external volume"`
//! found zero memories — while a memory named
//! `reference_launchd_tcc_external_volume` sat in the corpus — because those words
//! never appear contiguously. Measured over seven realistic multi-word queries,
//! six returned zero hits. A reader who types more than one word to narrow a
//! search got fewer results the more precisely they described what they wanted,
//! which is the exact opposite of the intended behaviour.
//!
//! **Single-word queries returned everything, in alphabetical order.** Only seven
//! scores are possible, so a term appearing in the body of forty memories put all
//! forty in one bucket, broken alphabetically: `"launchd"` returned 30 hits of
//! which 27 tied, ordered by nothing but the first letter of the filename.
//!
//! So: tokenise and require every term, then order by **BM25**. Raw term
//! frequency ranks the longest memories, and the long ones here are the project
//! notes, not the rules. BM25 saturates term frequency and normalises for length,
//! and its IDF term is what makes a multi-word query work: in "restic offsite
//! pull", "pull" is everywhere and "restic" is not, and the rare word should
//! decide.
use std::collections::HashMap;

use crate::couse::Usage;

/// Saturation: how fast extra occurrences stop helping. The standard 1.2.
const K1: f64 = 1.2;
/// Length normalisation, 0 = off, 1 = full. The standard 0.75.
const B: f64 = 0.75;

/// How much a term in the memory's NAME counts over one in its body.
///
/// The name is chosen, not written — `reference_launchd_tcc_external_volume`
/// is a deliberate statement of what the document is about, where the body may
/// mention launchd once in passing. Large, but not so large that a filename
/// keyword beats a memory that genuinely covers the subject.
const NAME_BOOST: f64 = 3.0;

/// The description is also chosen, and is a summary rather than a title.
const DESC_BOOST: f64 = 2.0;

/// Bonus for the query's terms appearing adjacently, as typed.
///
/// A memory containing "protocol version mismatch" answers better than one
/// mentioning protocols in one paragraph and versions in another. Multiplied
/// rather than added so it scales with the underlying relevance.
const PHRASE_BOOST: f64 = 1.8;

/// How much a prefix-only match counts against an exact one.
///
/// "backup" should find "backups", but should not find "background" as eagerly
/// as it finds "backu". Weighting rather than forbidding keeps the forgiving
/// behaviour while stopping a short term from scoring on every long word that
/// happens to start the same way.
const PREFIX_WEIGHT: f64 = 0.45;

/// Most-used memories get at most this multiplier over never-used ones.
///
/// Deliberately mild, and a tiebreaker rather than a ranking. What the reader
/// asked for comes first; how much the work leans on a memory only separates
/// answers that are otherwise comparable. Anything stronger would bury a
/// precise answer under a popular one, which is how a search stops being a
/// search and becomes a list of favourites.
const PRIOR_MAX: f64 = 0.35;

/// A term as searched: lowercase, alphanumeric, plus the joined form of any
/// hyphenated compound.
///
/// The join is what makes `"one-way VPN peer"` find `project_mac_oneway_vpn`.
/// Without it the query yields `["one", "way", "vpn", "peer"]` while the memory's
/// own name yields `["project", "mac", "oneway", "vpn"]` — so the two spellings of
/// one concept never meet, the name boost never fires for the very thing being
/// searched for, and the memory ranked fourth behind three that merely mention
/// VPNs. Applied to documents and queries alike, so `one-way` and `oneway` are
/// the same term whichever side writes which.
///
/// Hyphens only. Splitting on every separator and joining those too would fuse
/// `mysql.proc` into `mysqlproc` and glue sentences together across full stops,
/// inventing terms nobody wrote.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in text.split(|c: char| !c.is_alphanumeric() && c != '-') {
        let parts: Vec<String> = run
            .split('-')
            .filter(|p| !p.is_empty())
            .map(str::to_lowercase)
            .collect();
        if parts.is_empty() {
            continue;
        }
        if parts.len() > 1 {
            out.push(parts.concat());
        }
        out.extend(parts);
    }
    out
}

/// Exact-token hits and prefix-only hits, counted separately.
fn term_hits(tokens: &[String], term: &str) -> (usize, usize) {
    let mut exact = 0;
    let mut prefix = 0;
    for t in tokens {
        if t == term {
            exact += 1;
        } else if t.starts_with(term) {
            prefix += 1;
        }
    }
    (exact, prefix)
}

/// Effective term frequency, discounting inexact matches.
fn weighted(hits: (usize, usize)) -> f64 {
    hits.0 as f64 + hits.1 as f64 * PREFIX_WEIGHT
}

/// What the caller supplies per candidate memory.
pub struct Doc<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub body: &'a str,
    /// How much the work actually uses it, when a co-use artefact was mined.
    pub usage: Option<&'a Usage>,
}

/// One candidate's tokenised fields, and its total length for the BM25 norm.
struct Tokens {
    name: Vec<String>,
    description: Vec<String>,
    body: Vec<String>,
    len: usize,
}

/// A scored match.
#[derive(Debug, Clone, Copy)]
pub struct Scored {
    pub index: usize,
    pub score: f64,
}

/// How much the usage prior lifts one memory, in `[1, 1 + PRIOR_MAX]`.
///
/// Breadth of sessions rather than raw mentions: a memory hammered through one
/// long week is not more load-bearing than one consulted quietly in twenty
/// separate pieces of work, and mention counts are dominated by the former.
fn prior(usage: Option<&Usage>) -> f64 {
    let Some(u) = usage else { return 1.0 };
    1.0 + PRIOR_MAX * (u.sessions as f64).sqrt().min(3.6) / 3.6
}

/// Score every memory against `query`, best first.
///
/// Returns an empty vec when nothing carries every term — the caller decides
/// whether to relax, and must say so if it does. A search that quietly widens
/// its own query reports loose matches as though they were what was asked for.
pub fn rank(docs: &[Doc<'_>], query: &str, require_all: bool) -> Vec<Scored> {
    let terms = tokenize(query);
    if terms.is_empty() || docs.is_empty() {
        return Vec::new();
    }
    let phrase = query.trim().to_lowercase();

    // Pass one, over EVERY doc: pick the candidates and, in the same sweep, count
    // how many memories in the whole corpus carry each term.
    //
    // Document frequency MUST be corpus-wide. Measured over the candidates it is
    // degenerate by construction — requiring every term guarantees every candidate
    // has every term, so df == n, idf collapses to a constant, and the rarity
    // signal that motivated BM25 silently stops existing. "restic" is rare and
    // "pull" is common as a fact about the corpus, not about the five memories
    // that mention both.
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut candidates: Vec<usize> = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        let hay = format!("{} {} {}", doc.name, doc.description, doc.body).to_lowercase();
        // Hyphens stripped as well, so the joined form of a compound survives the
        // prefilter: a memory that writes "one-way" does not contain the
        // substring "oneway", and without this the tokeniser's joined term would
        // be admitted by nothing and the whole query would fail the AND.
        let joined = hay.replace('-', "");
        let mut all = true;
        let mut any = false;
        for term in &terms {
            if hay.contains(term.as_str()) || joined.contains(term.as_str()) {
                *df.entry(term.as_str()).or_insert(0) += 1;
                any = true;
            } else {
                all = false;
            }
        }
        if if require_all { all } else { any } {
            candidates.push(i);
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }

    // Pass two: tokenise the survivors only.
    let mut prepared: Vec<Tokens> = Vec::with_capacity(candidates.len());
    let mut total_len = 0usize;
    for &i in &candidates {
        let name = tokenize(docs[i].name);
        let description = tokenize(docs[i].description);
        let body = tokenize(docs[i].body);
        let len = name.len() + description.len() + body.len();
        total_len += len;
        prepared.push(Tokens {
            name,
            description,
            body,
            len,
        });
    }
    // Scored against the WHOLE corpus, so idf means what it says.
    let n = docs.len() as f64;
    let avg_len = (total_len as f64 / candidates.len() as f64).max(1.0);

    let mut out = Vec::new();
    for (slot, &i) in candidates.iter().enumerate() {
        let doc = &docs[i];
        let t = &prepared[slot];
        let mut score = 0.0;
        let mut matched_any = false;

        for term in &terms {
            let in_name = weighted(term_hits(&t.name, term));
            let in_desc = weighted(term_hits(&t.description, term));
            let in_body = weighted(term_hits(&t.body, term));
            // Field weights applied to the frequency, so a name hit behaves like
            // several body hits and still saturates rather than running away.
            let tf = in_name * NAME_BOOST + in_desc * DESC_BOOST + in_body;
            if tf <= 0.0 {
                continue;
            }
            matched_any = true;
            // Standard BM25 IDF with +0.5 smoothing, floored at zero: a term in
            // more than half the corpus would otherwise score negative and push
            // good matches down.
            let d = *df.get(term.as_str()).unwrap_or(&0) as f64;
            let idf = (((n - d + 0.5) / (d + 0.5)) + 1.0).ln().max(0.0);
            let norm = 1.0 - B + B * (t.len as f64 / avg_len);
            score += idf * (tf * (K1 + 1.0)) / (tf + K1 * norm);
        }

        // The substring prefilter is looser than the prefix-token rule used for
        // scoring — "vpn" sits inside "advpn" — so a candidate can reach here
        // with no scoring hit at all.
        if !matched_any {
            continue;
        }

        // The terms as typed, adjacent. Checked on the raw text rather than the
        // tokens so punctuation inside a phrase ("mysql.proc") still counts.
        if terms.len() > 1
            && (doc.name.to_lowercase().contains(&phrase)
                || doc.description.to_lowercase().contains(&phrase)
                || doc.body.to_lowercase().contains(&phrase))
        {
            score *= PHRASE_BOOST;
        }

        score *= prior(doc.usage);
        out.push(Scored { index: i, score });
    }

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

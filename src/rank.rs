//! Ranking for the memory search: tokenise, require every term, order by BM25.
//!
//! The substring match this replaced failed both ways: multi-word queries
//! returned nothing (six of seven realistic ones), since the words never appear
//! contiguously, and single-word queries returned everything in alphabetical
//! order, since only seven scores were possible. BM25 saturates term frequency,
//! normalises for length, and its IDF lets the rare word decide.
use std::collections::HashMap;

use crate::couse::Usage;

/// Saturation: how fast extra occurrences stop helping. The standard 1.2.
const K1: f64 = 1.2;
/// Length normalisation, 0 = off, 1 = full. The standard 0.75.
const B: f64 = 0.75;

/// How much a term in the memory's NAME counts over one in its body: the name is
/// chosen, not written. Large, but not so large that a filename keyword beats a
/// memory that genuinely covers the subject.
const NAME_BOOST: f64 = 3.0;

/// The description is also chosen, and is a summary rather than a title.
const DESC_BOOST: f64 = 2.0;

/// Bonus for the query's terms appearing adjacently, as typed. Multiplied rather
/// than added, so it scales with the underlying relevance.
const PHRASE_BOOST: f64 = 1.8;

/// How much a prefix-only match counts against an exact one: "backup" should
/// find "backups" but not "background" as eagerly.
const PREFIX_WEIGHT: f64 = 0.45;

/// Most-used memories get at most this multiplier over never-used ones. Mild: a
/// tiebreaker, or a search becomes a list of favourites.
const PRIOR_MAX: f64 = 0.35;

/// A term as searched: lowercase, alphanumeric, plus the joined form of any
/// hyphenated compound — what makes `"one-way VPN peer"` find
/// `project_mac_oneway_vpn`. Hyphens only: splitting on every separator would
/// fuse `mysql.proc` into `mysqlproc`.
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

/// How much the usage prior lifts one memory, in `[1, 1 + PRIOR_MAX]`. Breadth
/// of sessions rather than raw mentions.
fn prior(usage: Option<&Usage>) -> f64 {
    let Some(u) = usage else { return 1.0 };
    1.0 + PRIOR_MAX * (u.sessions as f64).sqrt().min(3.6) / 3.6
}

/// Score every memory against `query`, best first. Empty when nothing carries
/// every term — the caller decides whether to relax, and must say so.
pub fn rank(docs: &[Doc<'_>], query: &str, require_all: bool) -> Vec<Scored> {
    let terms = tokenize(query);
    if terms.is_empty() || docs.is_empty() {
        return Vec::new();
    }
    let phrase = query.trim().to_lowercase();

    // Pass one, over EVERY doc: pick the candidates and count corpus-wide document
    // frequency in the same sweep. It MUST be corpus-wide: over the candidates
    // df == n and the idf collapses to a constant.
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut candidates: Vec<usize> = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        let hay = format!("{} {} {}", doc.name, doc.description, doc.body).to_lowercase();
        // Hyphens stripped as well, so the joined form of a compound survives the
        // prefilter.
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
            // Standard BM25 IDF with +0.5 smoothing, floored at zero: a term in more than
            // half the corpus would otherwise score negative.
            let d = *df.get(term.as_str()).unwrap_or(&0) as f64;
            let idf = (((n - d + 0.5) / (d + 0.5)) + 1.0).ln().max(0.0);
            let norm = 1.0 - B + B * (t.len as f64 / avg_len);
            score += idf * (tf * (K1 + 1.0)) / (tf + K1 * norm);
        }

        // The substring prefilter is looser than the scoring rule — "vpn" sits inside
        // "advpn" — so a candidate can reach here with no scoring hit.
        if !matched_any {
            continue;
        }

        // The terms as typed, adjacent, checked on the raw text so punctuation inside a
        // phrase still counts.
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

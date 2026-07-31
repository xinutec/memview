//! Ranking for the history search.
//!
//! The naive version was not merely unordered, it was biased: it took the first
//! N matches in scan order and sorted only those by date. A search for "backup"
//! found 984 turns and returned 100 of them drawn from just two sessions,
//! because the cap filled up before the scan reached the others — and "which
//! session said it" is the exact question the page exists to answer.
//!
//! So every match is scored, and the top N are taken from the whole set.
//!
//! **BM25**, because the alternatives are worse in ways that matter here. Raw
//! term frequency ranks the longest replies; a turn that mentions "backup"
//! twelve times in four thousand words is not more about backup than one that
//! says it twice in a sentence. Pure recency answers "what did we do lately",
//! which is not what someone half-remembering a phrase is asking. BM25 handles
//! both — it saturates term frequency and normalises for length — and its
//! IDF term is what makes a multi-word query work at all: in "flat-field
//! correction", "correction" is common and "flat-field" is not, and the rare
//! word should decide the ranking.
use std::collections::HashMap;

/// Saturation: how fast extra occurrences stop helping. The standard 1.2.
const K1: f64 = 1.2;
/// Length normalisation, 0 = off, 1 = full. The standard 0.75.
const B: f64 = 0.75;

/// How much more a match in the reader's own words counts than one in a reply.
///
/// Asking about something is a stronger statement of topic than mentioning it:
/// a reply can name a thing in passing while answering about something else,
/// but a prompt naming it is what the turn was FOR. Not so large that a reply
/// match loses — the whole point of indexing replies is that "I remember it
/// saying something" is a real query.
const PROMPT_BOOST: f64 = 2.5;

/// Bonus for the query's terms appearing adjacently, as typed.
///
/// A turn containing "flat-field correction" is a better answer than one that
/// mentions flat-field in one paragraph and correction in another. Multiplied
/// rather than added so it scales with the underlying relevance.
const PHRASE_BOOST: f64 = 1.8;

/// Most recent turns get at most this multiplier over the oldest.
///
/// Deliberately mild. Recency is a tiebreaker between similar matches, not a
/// ranking: the corpus goes back to April and the thing being looked for is
/// often old, which is precisely why it is being looked for rather than
/// remembered.
const RECENCY_MAX: f64 = 1.3;

/// A term as searched: lowercase, alphanumeric.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// How much a prefix-only match counts against an exact one.
///
/// "backup" should find "backups", but it should not find "procedure" as
/// eagerly as it finds "proc". Weighting rather than forbidding keeps the
/// forgiving behaviour while stopping a short query term from scoring on every
/// long word that happens to start the same way.
const PREFIX_WEIGHT: f64 = 0.45;

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

/// What the caller must supply per candidate turn.
pub struct Doc<'a> {
    pub prompt: &'a str,
    pub reply: &'a str,
    /// Sort key for recency; any monotonic string (ISO-8601 works).
    pub at: &'a str,
}

/// A scored match.
#[derive(Debug, Clone, Copy)]
pub struct Scored {
    pub index: usize,
    pub score: f64,
    /// True when the query matched the prompt rather than only the reply.
    pub in_prompt: bool,
}

/// Score every document against `query`, best first.
///
/// Two passes because IDF needs document frequencies over the whole candidate
/// set: a term's weight depends on how rare it is, which is not knowable while
/// still counting. The corpus is ~15k turns, so both passes are trivial.
pub fn rank(docs: &[Doc<'_>], query: &str) -> Vec<Scored> {
    let terms = tokenize(query);
    if terms.is_empty() || docs.is_empty() {
        return Vec::new();
    }
    let phrase = query.trim().to_lowercase();

    // ALL terms, not any. An OR search reported "one-way VPN peer" as 9,297
    // matches because "one" and "way" are everywhere — and the per-session
    // tallies built from that count were noise, which defeats the one thing
    // they exist for. If nothing carries every term the caller retries relaxed.
    // Pass one, over EVERY document: pick the candidates and, in the same
    // sweep, count how many turns in the whole corpus carry each term.
    //
    // Document frequency MUST be corpus-wide. Measured over the candidates it
    // is degenerate by construction — AND guarantees every candidate contains
    // every term, so df == n, idf collapses to ~0.02, and the rarity signal
    // that motivated BM25 silently stops existing. "flat-field" is rare and
    // "correction" is common as a fact about the archive, not about the 23
    // turns that mention both.
    //
    // The substring test is deliberately looser than the prefix-token rule used
    // for scoring: it can over-count ("vpn" inside "advpn"), which nudges a
    // term's weight down slightly. Acceptable for a weight, and it costs
    // nothing — the lowercasing has to happen for the filter anyway.
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut candidates: Vec<usize> = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        let p = doc.prompt.to_lowercase();
        let r = doc.reply.to_lowercase();
        let mut all = true;
        for term in &terms {
            if p.contains(term.as_str()) || r.contains(term.as_str()) {
                *df.entry(term.as_str()).or_insert(0) += 1;
            } else {
                all = false;
            }
        }
        if all {
            candidates.push(i);
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }

    // Pass two: tokenise the survivors only. This is the expensive half, and
    // the filter above keeps it off the ~99% of turns that cannot match.
    let mut prepared: Vec<(Vec<String>, Vec<String>, usize)> = Vec::with_capacity(candidates.len());
    let mut total_len = 0usize;
    for &i in &candidates {
        let p = tokenize(docs[i].prompt);
        let r = tokenize(docs[i].reply);
        let len = p.len() + r.len();
        total_len += len;
        prepared.push((p, r, len));
    }
    // Scored against the WHOLE corpus, so idf and the length norm both mean
    // what they say.
    let n = docs.len() as f64;
    let avg_len = (total_len as f64 / candidates.len() as f64).max(1.0);

    // Recency is scaled across the observed span rather than against today, so
    // an archive that stops being written does not slowly flatten to no signal.
    let oldest = docs.iter().map(|d| d.at).min().unwrap_or("");
    let newest = docs.iter().map(|d| d.at).max().unwrap_or("");

    let mut out = Vec::new();
    for (slot, &i) in candidates.iter().enumerate() {
        let doc = &docs[i];
        let (p_tokens, r_tokens, len) = &prepared[slot];
        let mut score = 0.0;
        let mut in_prompt = false;
        let mut missing = false;

        for term in &terms {
            let in_p = weighted(term_hits(p_tokens, term));
            let in_r = weighted(term_hits(r_tokens, term));
            let tf = in_p + in_r;
            if tf <= 0.0 {
                // The substring prefilter admitted it, but no TOKEN starts with
                // the term — "vpn" inside "advpn". Under AND that disqualifies.
                missing = true;
                break;
            }
            if in_p > 0.0 {
                in_prompt = true;
            }
            // Standard BM25 IDF with the +0.5 smoothing, floored at zero: a
            // term present in more than half the corpus would otherwise score
            // negative and actively push good matches down.
            let d = *df.get(term.as_str()).unwrap_or(&0) as f64;
            let idf = (((n - d + 0.5) / (d + 0.5)) + 1.0).ln().max(0.0);
            let norm = 1.0 - B + B * (*len as f64 / avg_len);
            let mut term_score = idf * (tf * (K1 + 1.0)) / (tf + K1 * norm);
            if in_p > 0.0 {
                term_score *= PROMPT_BOOST;
            }
            score += term_score;
        }

        if missing {
            continue;
        }

        // The terms as typed, adjacent. Checked on the raw text rather than the
        // tokens so punctuation inside a phrase ("mysql.proc") still counts.
        if terms.len() > 1
            && (doc.prompt.to_lowercase().contains(&phrase)
                || doc.reply.to_lowercase().contains(&phrase))
        {
            score *= PHRASE_BOOST;
        }

        score *= recency_factor(doc.at, oldest, newest);
        out.push(Scored {
            index: i,
            score,
            in_prompt,
        });
    }

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// 1.0 for the oldest turn rising to [`RECENCY_MAX`] for the newest.
fn recency_factor(at: &str, oldest: &str, newest: &str) -> f64 {
    if at.is_empty() || oldest >= newest {
        return 1.0;
    }
    // Position by string comparison rather than date arithmetic: ISO-8601 sorts
    // correctly as text, and this needs an ordering, not a duration.
    let span = newest.len().max(at.len());
    let mut rank = 0.0;
    for i in 0..span {
        let a = at.as_bytes().get(i).copied().unwrap_or(0);
        let lo = oldest.as_bytes().get(i).copied().unwrap_or(0);
        let hi = newest.as_bytes().get(i).copied().unwrap_or(0);
        if lo != hi {
            rank = (a.saturating_sub(lo)) as f64 / (hi - lo) as f64;
            break;
        }
    }
    1.0 + rank.clamp(0.0, 1.0) * (RECENCY_MAX - 1.0)
}

/// Interleave by group so one session cannot fill the page.
///
/// The page's question is "WHICH session said this", and a straight top-N
/// answers it badly: "backup" is discussed in every session, so the highest
/// scores cluster in whichever one talked about it most and the answer looks
/// like one session when it is really nine. Round-robin over groups in score
/// order keeps the best hit from each session near the top while preserving
/// ranking inside a group.
pub fn diversify<T, K: Eq + std::hash::Hash + Clone>(
    items: Vec<T>,
    key: impl Fn(&T) -> K,
    limit: usize,
) -> Vec<T> {
    // Buckets keep first-seen order, and the input arrives sorted by score, so
    // bucket order IS "which session had the single best hit" — the round-robin
    // then preserves that as the order of the first page.
    let mut order: Vec<K> = Vec::new();
    let mut buckets: HashMap<K, std::collections::VecDeque<T>> = HashMap::new();
    for item in items {
        let k = key(&item);
        if !buckets.contains_key(&k) {
            order.push(k.clone());
        }
        buckets.entry(k).or_default().push_back(item);
    }

    let mut out = Vec::with_capacity(limit);
    while out.len() < limit {
        let mut placed = false;
        for k in &order {
            if out.len() >= limit {
                break;
            }
            if let Some(item) = buckets.get_mut(k).and_then(|b| b.pop_front()) {
                out.push(item);
                placed = true;
            }
        }
        // Every bucket empty: nothing left to interleave.
        if !placed {
            break;
        }
    }
    out
}

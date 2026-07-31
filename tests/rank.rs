//! Search ranking. Every assertion here is about which result comes FIRST,
//! because that is the only part of a search anybody reads.
use memview::rank::{Doc, diversify, rank, tokenize};

fn doc<'a>(prompt: &'a str, reply: &'a str, at: &'a str) -> Doc<'a> {
    Doc { prompt, reply, at }
}

#[test]
fn a_rare_term_decides_a_multi_word_query() {
    // The whole reason IDF is here. "correction" is everywhere in this corpus,
    // "flat-field" is not, so the turn with the rare word must win even though
    // the other repeats the common one.
    let docs = vec![
        doc(
            "",
            "a correction, another correction, correction again",
            "2026-07-01",
        ),
        doc("", "this is a flat-field correction", "2026-07-01"),
    ];
    let out = rank(&docs, "flat-field correction");
    assert_eq!(out[0].index, 1);
}

#[test]
fn asking_about_something_beats_being_told_about_it() {
    let docs = vec![
        doc(
            "",
            "we should check the shutter timing at some point",
            "2026-07-01",
        ),
        doc(
            "why does the shutter close?",
            "because of calibration",
            "2026-07-01",
        ),
    ];
    let out = rank(&docs, "shutter");
    assert_eq!(out[0].index, 1, "a prompt match should rank first");
}

#[test]
fn the_phrase_as_typed_beats_the_same_words_scattered() {
    let docs = vec![
        doc(
            "",
            "the vpn is one-way. separately, a peer was added.",
            "2026-07-01",
        ),
        doc("", "the mac is a one-way vpn peer", "2026-07-01"),
    ];
    let out = rank(&docs, "one-way vpn peer");
    assert_eq!(out[0].index, 1);
}

#[test]
fn a_long_reply_does_not_win_by_sheer_length() {
    // Without BM25's length normalisation the padded document wins purely for
    // being big, which is how a search ends up returning essays.
    let padding = "unrelated words ".repeat(400);
    let long = format!("backup {padding}");
    let docs = vec![
        doc("", &long, "2026-07-01"),
        doc("", "backup", "2026-07-01"),
    ];
    let out = rank(&docs, "backup");
    assert_eq!(out[0].index, 1, "the concise match should win");
}

#[test]
fn a_term_matches_a_words_prefix_but_not_its_middle() {
    // "backup" should find "backups"; "vpn" must NOT find "advpn", or a recall
    // search fills with accidental infixes.
    assert_eq!(
        rank(&[doc("", "the backups ran", "2026-07-01")], "backup").len(),
        1
    );
    assert!(rank(&[doc("", "an advpn tunnel", "2026-07-01")], "vpn").is_empty());
}

#[test]
fn every_term_must_be_present() {
    // OR semantics reported "one-way VPN peer" as 9,297 matches because "one"
    // and "way" are everywhere, and the per-session tallies built from that
    // count were noise — which defeats the only thing those tallies are for.
    let docs = vec![
        doc("", "the vpn is configured", "2026-07-01"),
        doc("", "a one-way vpn peer", "2026-07-01"),
    ];
    let out = rank(&docs, "one-way vpn peer");
    assert_eq!(out.len(), 1, "only the turn carrying every term matches");
    assert_eq!(out[0].index, 1);
}

#[test]
fn an_exact_match_outranks_a_merely_prefixed_one() {
    // "proc" should not score on "procedure" as hard as on "proc" itself, or a
    // short query term scores on every long word that starts the same way.
    let docs = vec![
        doc("", "follow the procedure carefully", "2026-07-01"),
        doc("", "check mysql proc now", "2026-07-01"),
    ];
    let out = rank(&docs, "proc");
    assert_eq!(out[0].index, 1);
}

#[test]
fn recency_breaks_a_tie_without_deciding_the_ranking() {
    let same = "the backup ran";
    let docs = vec![doc("", same, "2026-04-01"), doc("", same, "2026-07-31")];
    let out = rank(&docs, "backup");
    assert_eq!(
        out[0].index, 1,
        "newer wins when nothing else separates them"
    );

    // But a genuinely better old match must still beat a weak new one.
    let docs = vec![
        doc("tell me about the backup", "the backup ran", "2026-04-01"),
        doc("", "unrelated, though a backup exists", "2026-07-31"),
    ];
    let out = rank(&docs, "backup");
    assert_eq!(out[0].index, 0, "relevance must outrank recency");
}

#[test]
fn a_document_matching_nothing_is_absent_rather_than_last() {
    let docs = vec![doc("", "nothing relevant here", "2026-07-01")];
    assert!(rank(&docs, "backup").is_empty());
}

#[test]
fn an_empty_query_ranks_nothing() {
    let docs = vec![doc("", "anything", "2026-07-01")];
    assert!(rank(&docs, "").is_empty());
    assert!(rank(&docs, "   ").is_empty());
}

#[test]
fn tokenizing_splits_on_punctuation_and_lowercases() {
    assert_eq!(
        tokenize("mysql.proc  Column-Count"),
        ["mysql", "proc", "column", "count"]
    );
    assert!(tokenize("...").is_empty());
}

#[test]
fn diversify_interleaves_so_one_group_cannot_fill_the_page() {
    // The defect this exists for: "backup" returned 100 rows from two sessions
    // out of nine, because the best-scoring session monopolised the page.
    let items: Vec<(&str, u32)> = vec![
        ("home", 1),
        ("home", 2),
        ("home", 3),
        ("home", 4),
        ("recall", 5),
        ("health", 6),
    ];
    let out = diversify(items, |(s, _)| *s, 4);
    let groups: Vec<&str> = out.iter().map(|(s, _)| *s).collect();
    assert_eq!(groups, ["home", "recall", "health", "home"]);
}

#[test]
fn diversify_keeps_score_order_inside_a_group_and_across_first_picks() {
    // Bucket order is first-seen, and the input arrives sorted by score, so the
    // first row of each group appears in the order those groups' best hits did.
    let items: Vec<(&str, u32)> = vec![("b", 1), ("a", 2), ("b", 3), ("a", 4)];
    let out = diversify(items, |(s, _)| *s, 4);
    assert_eq!(out, [("b", 1), ("a", 2), ("b", 3), ("a", 4)]);
}

#[test]
fn diversify_drains_everything_when_the_limit_allows() {
    let items: Vec<(&str, u32)> = vec![("a", 1), ("a", 2), ("b", 3)];
    assert_eq!(diversify(items, |(s, _)| *s, 99).len(), 3);
}

#[test]
fn rarity_is_measured_against_the_whole_corpus_not_the_matches() {
    // The subtle failure this guards: under AND every candidate contains every
    // term, so document frequency measured over the candidates is always the
    // candidate count, idf collapses to ~0, and BM25 degenerates into raw term
    // frequency. Here "correction" is corpus-common and "flatfield" is rare, so
    // the turn carrying the rare word must win — which can only happen if df
    // was counted over all six documents rather than the two that match both.
    let common = doc("", "a correction was made", "2026-07-01");
    let docs = vec![
        doc(
            "",
            "correction correction correction and flatfield",
            "2026-07-01",
        ),
        doc("", "flatfield correction", "2026-07-01"),
        // Corpus context: "correction" is everywhere, "flatfield" is not.
        doc("", common.reply, "2026-07-01"),
        doc("", common.reply, "2026-07-01"),
        doc("", common.reply, "2026-07-01"),
        doc("", common.reply, "2026-07-01"),
    ];
    let out = rank(&docs, "flatfield correction");
    assert_eq!(out.len(), 2, "only two turns carry both terms");
    // Both matched; the scores must be non-trivial rather than idf-flattened.
    assert!(out[0].score > 0.5, "idf collapsed: score {}", out[0].score);
}

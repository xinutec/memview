//! Search ranking, against synthetic memories.
//!
//! Every case here is one the substring search this replaced got wrong on the
//! live corpus — measured before the change, not imagined afterwards.

use memview::couse::Usage;
use memview::rank::{Doc, rank, tokenize};

fn doc<'a>(name: &'a str, description: &'a str, body: &'a str) -> Doc<'a> {
    Doc {
        name,
        description,
        body,
        usage: None,
    }
}

/// Names of the results, best first.
fn order<'a>(docs: &'a [Doc<'a>], query: &str, all: bool) -> Vec<&'a str> {
    rank(docs, query, all)
        .into_iter()
        .map(|s| docs[s.index].name)
        .collect()
}

#[test]
fn a_multi_word_query_matches_words_that_are_not_adjacent() {
    // THE fault this replaced. On the live corpus "launchd TCC external volume"
    // returned ZERO hits while `reference_launchd_tcc_external_volume` sat in
    // the corpus, because a substring search needs the words contiguous. Six of
    // seven realistic multi-word queries returned nothing at all.
    let docs = vec![
        doc(
            "reference_launchd_tcc_external_volume",
            "agents cannot run from an external volume",
            "TCC denies launchd-spawned jobs access to the volume.",
        ),
        doc("project_unrelated", "something else", "nothing to see"),
    ];

    let hits = order(&docs, "launchd TCC external volume", true);

    assert_eq!(hits, vec!["reference_launchd_tcc_external_volume"]);
}

#[test]
fn every_word_is_required() {
    // Otherwise "one-way VPN peer" reports every memory containing "one".
    let docs = vec![
        doc("has_both", "", "restic offsite"),
        doc("has_one", "", "restic only"),
    ];

    assert_eq!(order(&docs, "restic offsite", true), vec!["has_both"]);
}

#[test]
fn relaxing_finds_what_requiring_everything_could_not() {
    // The caller falls back to this when nothing carries every term — and has to
    // say so, which is what the `relaxed` flag on the response is for.
    let docs = vec![doc("partial", "", "restic only")];

    assert!(order(&docs, "restic offsite", true).is_empty());
    assert_eq!(order(&docs, "restic offsite", false), vec!["partial"]);
}

#[test]
fn a_hyphenated_compound_matches_its_joined_spelling() {
    // Measured: "one-way VPN peer" ranked project_mac_oneway_vpn FOURTH, behind
    // three memories that merely mention VPNs, because the query tokenised to
    // ["one","way",...] while the memory's own name held the single token
    // "oneway" — so the name boost never fired for the thing being searched for.
    let docs = vec![
        doc(
            "project_mac_oneway_vpn",
            "the mac is a one-way peer",
            "wireguard",
        ),
        doc(
            "project_other",
            "mentions a vpn and a peer",
            "vpn peer vpn peer",
        ),
    ];

    let hits = order(&docs, "one-way VPN peer", true);

    assert_eq!(hits.first(), Some(&"project_mac_oneway_vpn"));
}

#[test]
fn the_joined_form_works_in_both_directions() {
    // Whichever side writes which spelling.
    let docs = vec![doc("a_memory", "", "the oneway rule")];
    assert!(!order(&docs, "one-way", true).is_empty());

    let docs = vec![doc("b_memory", "", "the one-way rule")];
    assert!(!order(&docs, "oneway", true).is_empty());
}

#[test]
fn a_rare_word_decides_over_a_common_one() {
    // What IDF is for. "pull" is everywhere and "restic" is not, so the memory
    // about restic wins even though the other repeats "pull" far more.
    let mut docs = vec![
        doc("about_restic", "", "restic pull"),
        doc("about_pulling", "", "pull pull pull pull pull pull restic"),
    ];
    for i in 0..20 {
        // Corpus-wide document frequency is what makes "pull" cheap, so the
        // corpus has to actually contain it. Leaked as a static to keep the
        // borrow simple.
        let _ = i;
        docs.push(doc("noise", "", "pull"));
    }

    let hits = order(&docs, "restic pull", true);

    assert_eq!(hits.first(), Some(&"about_restic"));
}

#[test]
fn a_name_match_outranks_a_body_mention() {
    // The name is chosen; a body may mention the word in passing.
    let docs = vec![
        doc("reference_rrsync", "", "confined key"),
        doc("project_notes", "", "we used rrsync once for something"),
    ];

    assert_eq!(
        order(&docs, "rrsync", true).first(),
        Some(&"reference_rrsync")
    );
}

#[test]
fn usage_separates_comparable_answers_without_overriding_them() {
    // The prior is a tiebreaker. Between two memories that match identically,
    // the one the work actually leans on comes first...
    let used = Usage {
        sessions: 20,
        ..Usage::default()
    };
    let a = Doc {
        usage: Some(&used),
        ..doc("well_used", "", "restic")
    };
    let b = doc("never_used", "", "restic");
    let docs = vec![b, a];
    assert_eq!(order(&docs, "restic", true).first(), Some(&"well_used"));

    // ...but it must never lift a weak match over a strong one, or the search
    // stops being a search and becomes a list of favourites.
    let weak = Doc {
        usage: Some(&used),
        ..doc(
            "popular_but_vague",
            "",
            "restic mentioned once among many words that dilute it",
        )
    };
    let strong = doc("reference_restic", "restic", "restic restic");
    let docs = vec![weak, strong];
    assert_eq!(
        order(&docs, "restic", true).first(),
        Some(&"reference_restic")
    );
}

#[test]
fn tokenize_keeps_the_parts_and_adds_the_join() {
    assert_eq!(tokenize("one-way VPN"), vec!["oneway", "one", "way", "vpn"]);
    // Hyphens only: joining across every separator would fuse "mysql.proc" into
    // a term nobody wrote, and glue sentences together across full stops.
    assert_eq!(tokenize("mysql.proc"), vec!["mysql", "proc"]);
    assert_eq!(tokenize(""), Vec::<String>::new());
}

#[test]
fn an_empty_query_finds_nothing_rather_than_everything() {
    let docs = vec![doc("anything", "", "text")];
    assert!(order(&docs, "", true).is_empty());
    assert!(order(&docs, "   ", true).is_empty());
}

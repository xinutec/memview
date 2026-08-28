//! Text claims about tickets, checked against the service (#1179, #1227).

use std::collections::BTreeSet;

use memview::cites::{citations, is_ours, still_asks};

fn ours() -> BTreeSet<String> {
    ["memview", "life", "recall"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// The ids of ours a body cites, sorted — the set orders by qualifier first, and
/// that ordering is an implementation detail no test should depend on.
fn ids(body: &str) -> Vec<u64> {
    let mut out: Vec<u64> = citations(body)
        .into_iter()
        .filter(|c| is_ours(c, &ours()))
        .map(|c| c.id)
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn a_bare_id_and_one_qualified_by_our_own_name_are_both_citations() {
    assert_eq!(
        ids("see #1179 and memview#884 for the argument"),
        [884, 1179]
    );
}

/// ⚠ **Another project's tracker is not ours to check.** Measured 2026-08-28,
/// the first version of this reported exactly two dangling ids corpus-wide and
/// BOTH were this: `rxdb#7804` and `angular/components#33091`, real issues in
/// other people's repositories, cited correctly, reported as corpus rot. On a
/// check whose whole yield is about five, two false positives is the difference
/// between a signal and a nuisance.
#[test]
fn another_projects_issue_number_is_not_ours_to_resolve() {
    assert!(ids("replication meta (rxdb#7804), so local wins").is_empty());
    assert!(ids("angular/components#33091 is the upstream issue").is_empty());
}

/// ⚠ A markdown heading opens with `#` and a space. Reading it as a citation
/// would make every structured memory cite a ticket it never mentions.
#[test]
fn a_heading_is_not_a_citation() {
    assert!(ids("## The build\n\n### Why").is_empty());
}

/// ⚠ A CSS colour is a `#` and six characters. `#1a2b3c` carries letters, and
/// `#123456` does not — so length refuses it, because a false dangling report
/// costs more than a missed one on a check whose whole yield is about five.
#[test]
fn a_colour_is_not_a_citation() {
    assert!(ids("background: #1a2b3c").is_empty());
    assert!(ids("background: #123456").is_empty());
}

#[test]
fn a_version_or_range_is_not_a_citation() {
    assert!(ids("in 2026#12 style").is_empty());
}

/// The two cases #1227 measured: a closed ticket whose subject still says the
/// answer is not known.
#[test]
fn a_subject_that_still_asks_its_question_is_caught() {
    assert!(still_asks("the zombie's origin is still unknown"));
    assert!(still_asks(
        "Ticket ids have an oracle and nothing uses it — resolve #N in the nightly"
    ));
}

/// ⚠ **It must not reach past the one class with an oracle.** A subject naming a
/// measurement, or one whose body contradicts it, is not mechanically checkable
/// — and a rule that guessed would fire on most of the corpus.
#[test]
fn a_subject_that_merely_states_a_finding_is_left_alone() {
    assert!(!still_asks("875 file operations are unaccounted for"));
    assert!(!still_asks(
        "memory-tiers would demote tripwires — propose()'s demote filter never reads role"
    ));
    assert!(!still_asks(
        "A corpus grep is an open of what it MATCHED — counting all or nothing are both wrong"
    ));
}

//! Co-use mining, against synthetic transcripts.
//!
//! Every one of these pins a way the first working version lied. The numbers it
//! produced all looked plausible; three of them were artefacts of how the data
//! is shaped rather than facts about the memory, and none would have been
//! noticed by reading the ranking.

use std::collections::BTreeSet;
use std::path::PathBuf;

use memview::couse::{MIN_SESSIONS, scan};

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// A message line. `parent` of None makes it a root.
fn msg(uuid: &str, parent: Option<&str>, prompt: Option<&str>, text: &str) -> String {
    let parent = parent
        .map(|p| format!(",\"parentUuid\":\"{p}\""))
        .unwrap_or_default();
    let prompt = prompt
        .map(|p| format!(",\"promptId\":\"{p}\""))
        .unwrap_or_default();
    format!(
        "{{\"type\":\"assistant\",\"uuid\":\"{uuid}\"{parent}{prompt},\"message\":{{\"content\":\"{text}\"}}}}"
    )
}

/// Write sessions to a fresh directory and mine them.
fn mine(sessions: &[Vec<String>], corpus: &[&str]) -> memview::couse::CoUse {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "couse-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (i, lines) in sessions.iter().enumerate() {
        std::fs::write(dir.join(format!("s{i}.jsonl")), lines.join("\n")).unwrap();
    }
    let found = scan(&dir, &names(corpus)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    found
}

/// Three sessions in which `alpha` and `beta` meet inside one turn.
fn met_in_three() -> Vec<Vec<String>> {
    (0..3)
        .map(|s| {
            vec![
                msg(&format!("{s}a"), None, Some("p1"), "project_alpha"),
                msg(
                    &format!("{s}b"),
                    Some(&format!("{s}a")),
                    None,
                    "reference_beta",
                ),
            ]
        })
        .collect()
}

#[test]
fn a_pair_that_meets_across_sessions_is_reported() {
    let found = mine(&met_in_three(), &["project_alpha", "reference_beta"]);
    assert_eq!(found.pairs.len(), 1, "{:?}", found.pairs);
    assert_eq!(found.pairs[0].sessions, 3);
}

#[test]
fn a_message_inherits_its_turn_from_its_parent() {
    // Only the first message of each session carries a promptId; the second is
    // in the same turn solely by being its child. Without the inheritance the
    // two would land in different turns and never be seen to meet.
    let found = mine(&met_in_three(), &["project_alpha", "reference_beta"]);
    assert_eq!(found.pairs.len(), 1);
    assert_eq!(found.pairs[0].turns, 3);
}

#[test]
fn two_turns_of_the_same_session_are_not_a_meeting() {
    // The whole reason the turn is the unit. These two memories are in one
    // session but different requests, so they were never being thought about
    // together — a session-wide window would wrongly pair them.
    let sessions: Vec<Vec<String>> = (0..3)
        .map(|s| {
            vec![
                msg(&format!("{s}a"), None, Some("p1"), "project_alpha"),
                msg(&format!("{s}b"), None, Some("p2"), "reference_beta"),
            ]
        })
        .collect();
    let found = mine(&sessions, &["project_alpha", "reference_beta"]);
    assert!(found.pairs.is_empty(), "{:?}", found.pairs);
}

#[test]
fn a_turn_is_read_from_the_tree_and_not_from_file_order() {
    // The discriminating case, and the reason this walks parentUuid instead of
    // carrying the last-seen promptId forward.
    //
    // The file order is: root(p1) with alpha, then an UNRELATED turn p2, then a
    // continuation of root carrying beta. Read in order, beta lands in p2 and
    // meets gamma — a meeting that never happened. Read through the tree, beta
    // inherits p1 from its parent and meets alpha, which is what occurred.
    //
    // This is not exotic: it is what a transcript looks like whenever a prompt
    // is edited and re-run, or a branch is resumed after later messages exist.
    let sessions: Vec<Vec<String>> = (0..3)
        .map(|s| {
            vec![
                msg(&format!("{s}root"), None, Some("p1"), "project_alpha"),
                msg(&format!("{s}other"), None, Some("p2"), "project_gamma"),
                msg(
                    &format!("{s}beta"),
                    Some(&format!("{s}root")),
                    None,
                    "reference_beta",
                ),
            ]
        })
        .collect();
    let found = mine(
        &sessions,
        &["project_alpha", "reference_beta", "project_gamma"],
    );
    let pairs: Vec<(String, String)> = found
        .pairs
        .iter()
        .map(|p| (p.a.clone(), p.b.clone()))
        .collect();
    assert_eq!(
        pairs,
        vec![("project_alpha".to_string(), "reference_beta".to_string())],
        "beta must meet its parent's turn, not the turn that precedes it in the file"
    );
}

#[test]
fn a_pair_confined_to_one_session_is_not_reported() {
    // The artefact this exists for: one health-sync week produced 710 turns of a
    // single pair, which outranked every real relationship in the corpus. Turns
    // inside one session are one piece of work, not independent evidence.
    let mut only_one = vec![Vec::new()];
    for i in 0..50 {
        only_one[0].push(msg(
            &format!("t{i}"),
            None,
            Some(&format!("p{i}")),
            "project_alpha",
        ));
        only_one[0].push(msg(
            &format!("u{i}"),
            Some(&format!("t{i}")),
            None,
            "reference_beta",
        ));
    }
    let found = mine(&only_one, &["project_alpha", "reference_beta"]);
    assert!(
        found.pairs.is_empty(),
        "50 turns of one session must not outvote {MIN_SESSIONS} separate ones: {:?}",
        found.pairs
    );
}

#[test]
fn a_line_that_lists_many_memories_is_not_a_thought() {
    // MEMORY.md groups related memories onto single bullets, one of which names
    // seventeen, and the index reaches every session. Counted, those seventeen
    // manufacture 136 pairs that outranked everything genuine.
    let listing: String = (0..12)
        .map(|i| format!("memory_{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let corpus: Vec<String> = (0..12).map(|i| format!("memory_{i}")).collect();
    let refs: Vec<&str> = corpus.iter().map(String::as_str).collect();
    let sessions: Vec<Vec<String>> = (0..3)
        .map(|s| vec![msg(&format!("{s}a"), None, Some("p1"), &listing)])
        .collect();
    let found = mine(&sessions, &refs);
    assert!(found.pairs.is_empty(), "{:?}", found.pairs);
}

#[test]
fn a_name_the_corpus_does_not_have_is_ignored() {
    // memview's own test fixtures live at tests/fixtures/memory/*.md and match
    // every path pattern a real memory does. Unfiltered, they took the top three
    // places in the first ranking — they co-occur in every run of the suite,
    // which is a fact about the harness.
    let sessions: Vec<Vec<String>> = (0..3)
        .map(|s| {
            vec![msg(
                &format!("{s}a"),
                None,
                Some("p1"),
                "project_alpha project_fixture_only",
            )]
        })
        .collect();
    let found = mine(&sessions, &["project_alpha"]);
    assert!(found.pairs.is_empty(), "{:?}", found.pairs);
}

#[test]
fn unlinked_leaves_out_pairs_the_corpus_already_links() {
    let found = mine(&met_in_three(), &["project_alpha", "reference_beta"]);
    let linked: BTreeSet<(String, String)> =
        [("project_alpha".to_string(), "reference_beta".to_string())]
            .into_iter()
            .collect();
    assert!(found.unlinked(&linked).is_empty());
    assert_eq!(found.unlinked(&BTreeSet::new()).len(), 1);
}

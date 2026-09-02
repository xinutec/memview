//! Co-use mining, against synthetic transcripts.
//!
//! Every one of these pins a way the first working version lied. The numbers it
//! produced all looked plausible; three of them were artefacts of how the data
//! is shaped rather than facts about the memory, and none would have been
//! noticed by reading the ranking.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use memview::couse::{MIN_SESSIONS, scan};

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// A message line. `parent` of None makes it a root.
fn msg(uuid: &str, parent: Option<&str>, prompt: Option<&str>, text: &str) -> String {
    msg_in(uuid, parent, prompt, text, None)
}

/// The same, in a working directory — which is how a mention gets attributed
/// to a project.
fn msg_in(
    uuid: &str,
    parent: Option<&str>,
    prompt: Option<&str>,
    text: &str,
    cwd: Option<&str>,
) -> String {
    let parent = parent
        .map(|p| format!(",\"parentUuid\":\"{p}\""))
        .unwrap_or_default();
    let prompt = prompt
        .map(|p| format!(",\"promptId\":\"{p}\""))
        .unwrap_or_default();
    let cwd = cwd.map(|c| format!(",\"cwd\":\"{c}\"")).unwrap_or_default();
    format!(
        "{{\"type\":\"assistant\",\"uuid\":\"{uuid}\"{parent}{prompt}{cwd},\"message\":{{\"content\":\"{text}\"}}}}"
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
    let found = scan(&dir, &names(corpus), "/code", "2026-07-31T00:00:00Z").unwrap();
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

/// Adjacency from unordered pairs, the shape `lint::check` builds.
fn adjacency(edges: &[(&str, &str)]) -> BTreeMap<String, BTreeSet<String>> {
    let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (a, b) in edges {
        adj.entry(a.to_string()).or_default().insert(b.to_string());
        adj.entry(b.to_string()).or_default().insert(a.to_string());
    }
    adj
}

#[test]
fn unlinked_leaves_out_pairs_the_corpus_already_links() {
    let found = mine(&met_in_three(), &["project_alpha", "reference_beta"]);
    let (missing, connected) = found.unlinked(&adjacency(&[("project_alpha", "reference_beta")]));
    assert!(missing.is_empty());
    assert_eq!(connected, 1);

    let (missing, connected) = found.unlinked(&BTreeMap::new());
    assert_eq!(missing.len(), 1);
    assert_eq!(connected, 0);
}

/// A hub linking both is a connection, so its children are not a missing link.
///
/// The case this was written for: four memories split out of one roadmap on
/// 2026-08-22 co-occur at npmi 1.00 and link only their parent. Demanding a link
/// between them would ask a well-formed hub to become a clique.
#[test]
fn unlinked_leaves_out_two_children_of_one_hub() {
    let found = mine(&met_in_three(), &["project_alpha", "reference_beta"]);
    let (missing, connected) = found.unlinked(&adjacency(&[
        ("project_hub", "project_alpha"),
        ("project_hub", "reference_beta"),
    ]));
    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(connected, 1);
}

/// One shared neighbour, not connectivity: a two-hop chain is still a finding.
#[test]
fn unlinked_still_reports_a_pair_joined_only_by_a_chain() {
    let found = mine(&met_in_three(), &["project_alpha", "reference_beta"]);
    let (missing, _) = found.unlinked(&adjacency(&[
        ("project_alpha", "project_middle_one"),
        ("project_middle_one", "project_middle_two"),
        ("project_middle_two", "reference_beta"),
    ]));
    assert_eq!(missing.len(), 1);
}

// -- project attribution ---------------------------------------------------

#[test]
fn a_mention_is_credited_to_the_directory_it_was_made_in() {
    // Two sessions consulting the same memory in different repositories. The
    // document says nothing about either — `cwd` is the only evidence, which is
    // exactly why it is the signal: MEMORY.md names every project and is
    // injected everywhere, so any text test would match both.
    let sessions = vec![
        vec![
            msg_in(
                "a0",
                None,
                Some("p1"),
                "project_alpha",
                Some("/code/health/src"),
            ),
            msg_in("a1", Some("a0"), Some("p1"), "reference_beta", None),
        ],
        vec![
            msg_in("b0", None, Some("p2"), "project_alpha", Some("/code/life")),
            msg_in("b1", Some("b0"), Some("p2"), "reference_beta", None),
        ],
    ];
    let found = mine(&sessions, &["project_alpha", "reference_beta"]);
    let alpha = &found.usage["project_alpha"];
    assert_eq!(alpha.projects.get("health"), Some(&1));
    assert_eq!(alpha.projects.get("life"), Some(&1));
    // A line with no cwd contributes no project rather than a guessed one: an
    // unattributed mention must not inflate whichever project happens to be
    // nearby, or the clustering it feeds would be confidently wrong.
    assert!(found.usage["reference_beta"].projects.is_empty());
}

#[test]
fn work_outside_the_code_root_has_no_project() {
    // Not every session is in a repository — the corpus itself lives elsewhere.
    // None, rather than the last path element, which would invent projects
    // named after home directories and scratch folders.
    assert_eq!(
        memview::couse::project_of("/code/health", "/code"),
        Some("health".into())
    );
    assert_eq!(
        memview::couse::project_of("/code/health/a/b", "/code"),
        Some("health".into())
    );
    assert_eq!(
        memview::couse::project_of("/elsewhere/health", "/code"),
        None
    );
    assert_eq!(memview::couse::project_of("/code", "/code"), None);
    // A trailing slash on the root must not change the answer.
    assert_eq!(
        memview::couse::project_of("/code/health", "/code/"),
        Some("health".into())
    );
}

/// Support admits a pair; npmi judges it. Two memories that are EVERYWHERE and
/// meet in only a few sessions co-occur less than chance would give — that is
/// not a habit, and keeping such pairs made 78% of the live artefact weightless
/// (memview#1307): counted, drawn as companions, ignored by the layout.
#[test]
fn a_below_chance_pair_is_dropped_at_the_mine() {
    // Three sessions where alpha and beta meet inside one turn…
    let mut sessions = met_in_three();
    // …and six more where both are PRESENT but never meet: each meets gamma in
    // its own turn instead. ⚠ Presence has to be staged through a meeting,
    // because a single-name turn never reaches a basket (`b.len() >= 2` in
    // scan_session) — the first version of this test put alpha and beta in
    // solo turns and their presence simply vanished, leaving pab = 1. With
    // gamma: pa = pb = 1, pab = 3/9 — below chance, npmi < 0, dropped. And so
    // are both gamma pairs, for the same reason: a name in EVERY session
    // leaves nothing for chance to beat.
    for s in 3..9 {
        sessions.push(vec![
            msg(&format!("{s}a"), None, Some("p1"), "project_alpha"),
            msg(
                &format!("{s}b"),
                Some(&format!("{s}a")),
                None,
                "project_gamma",
            ),
            msg(&format!("{s}c"), None, Some("p2"), "reference_beta"),
            msg(
                &format!("{s}d"),
                Some(&format!("{s}c")),
                None,
                "project_gamma",
            ),
        ]);
    }
    let found = mine(
        &sessions,
        &["project_alpha", "reference_beta", "project_gamma"],
    );
    assert!(
        found.pairs.is_empty(),
        "a pair everyone has and few use together is not an affinity: {:?}",
        found.pairs
    );
}

/// The degenerate end of the same formula: a pair present in EVERY session has
/// pab = 1 and the formula reads 0/0. That pair is the strongest this measure
/// can support, so it is 1 by definition — NaN would fail the below-chance
/// filter and silently drop the best pair in a small corpus.
#[test]
fn a_pair_present_in_every_session_scores_one_not_nan() {
    let found = mine(&met_in_three(), &["project_alpha", "reference_beta"]);
    assert_eq!(found.pairs.len(), 1);
    assert_eq!(found.pairs[0].npmi, 1.0);
}

//! The document-graph rules: whether the corpus is navigable at all.

use memview::lint::{Severity, check, passed_for_session};
use memview::store::Corpus;

/// A corpus of an index plus `(name, body)` memories.
fn corpus(dir: &std::path::Path, index: &str, docs: &[(&str, &str)]) -> Corpus {
    std::fs::write(dir.join("MEMORY.md"), index).expect("write index");
    for (name, body) in docs {
        std::fs::write(
            dir.join(format!("{name}.md")),
            format!(
                "---\nname: {name}\ndescription: d\nmetadata:\n  type: project\n---\n\n{body}\n"
            ),
        )
        .expect("write memory");
    }
    Corpus::load(dir).expect("loads")
}

fn findings(corpus: &Corpus, rule: &str) -> Vec<String> {
    check(corpus, None)
        .into_iter()
        .filter(|f| f.rule == rule && f.severity == Severity::Error)
        .map(|f| f.memory)
        .collect()
}

#[test]
fn a_memory_several_hops_from_the_index_is_reachable() {
    // Pippijn, 2026-08-02: "MEMORY.md doesn't need to index everything. things
    // have to be reachable, but don't need to all be in MEMORY.md". The rule
    // used to demand an index line per memory and failed the gate on a corpus
    // that was perfectly navigable — three memories had just been consolidated
    // under one entry that links them all.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "see [[project_middle]]"),
            ("project_middle", "and [[project_leaf]]"),
            ("project_leaf", "the end, back to [[project_hub]]"),
        ],
    );
    assert!(findings(&found, "unreachable").is_empty());
}

#[test]
fn a_memory_nothing_links_is_an_error() {
    // The failure the rule is actually for: a memory that exists and that no
    // reader can arrive at.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "see [[project_middle]]"),
            ("project_middle", "nothing further"),
            (
                "project_island",
                "linked from nowhere, links [[project_hub]]",
            ),
        ],
    );
    // It links out, so it is not `stranded` — and it still cannot be found.
    assert_eq!(findings(&found, "unreachable"), ["project_island"]);
}

/// Findings at any severity — this rule was promoted to ERROR once the corpus
/// reached zero, so the tests must not assert a severity that can move again.
fn reported(corpus: &Corpus, rule: &str) -> Vec<String> {
    check(corpus, None)
        .into_iter()
        .filter(|f| f.rule == rule)
        .map(|f| f.memory)
        .collect()
}

#[test]
fn a_rule_the_work_it_governs_never_names_is_reported() {
    // The real 2026-08-15 case, reduced: two rules declared they governed the
    // life app, the hub named one of them, and a session rewrote that app's
    // emotion feature without ever seeing the other. Nothing was dangling,
    // stranded or unreachable — every other rule passed on this shape.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "style is [[feedback_style]]"),
            ("feedback_style", "[[governs:project_hub]]"),
            ("feedback_words", "[[governs:project_hub]]"),
        ],
    );
    assert_eq!(reported(&found, "governs-unreciprocated"), ["project_hub"]);
    // Filed against the hub, which is the file that has to change — not against
    // the rule, which already declared what it binds.
    assert!(
        check(&found, None)
            .iter()
            .any(|f| f.rule == "governs-unreciprocated" && f.detail.contains("feedback_words"))
    );
}

#[test]
fn naming_the_rule_back_settles_it_whatever_the_relation() {
    // The fix is a link, not a matching `governs` in the other direction: the
    // hub is a map, so a plain mention is exactly what it should carry.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "bound by [[feedback_style]]"),
            ("feedback_style", "[[governs:project_hub]]"),
        ],
    );
    assert!(reported(&found, "governs-unreciprocated").is_empty());
}

#[test]
fn part_of_and_because_are_not_held_to_this() {
    // Only `governs` is checked. Demanding a parent enumerate every child would
    // turn a map into an index, and `because` points at a reason that has no
    // business knowing what cites it.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "nothing points back"),
            ("project_child", "[[part-of:project_hub]]"),
            ("reference_fact", "[[because:project_hub]]"),
        ],
    );
    assert!(reported(&found, "governs-unreciprocated").is_empty());
}

#[test]
fn a_hub_may_delegate_its_rule_list_to_a_part_of_child() {
    // project_dicom_scan keeps twelve rules in project_dicom_scan_rules rather
    // than inline. That is better organisation, and the first draft of this rule
    // called all nine of its governors violations — the same error `unreachable`
    // made when it demanded an index line from a navigable corpus.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "the rules are [[project_hub_rules]]"),
            (
                "project_hub_rules",
                "[[part-of:project_hub]] and [[feedback_style]]",
            ),
            ("feedback_style", "[[governs:project_hub]]"),
        ],
    );
    assert!(reported(&found, "governs-unreciprocated").is_empty());
}

#[test]
fn delegation_is_one_hop_and_only_through_part_of() {
    // A grandchild is not the hub's answer, and a child that never declared
    // itself part of the hub is just another memory that happens to link it.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "[[project_mid]] and [[project_bystander]]"),
            (
                "project_mid",
                "[[part-of:project_hub]], see [[project_leaf]]",
            ),
            ("project_leaf", "[[part-of:project_mid]], [[feedback_deep]]"),
            ("project_bystander", "mentions [[feedback_shallow]]"),
            ("feedback_deep", "[[governs:project_hub]]"),
            ("feedback_shallow", "[[governs:project_hub]]"),
        ],
    );
    // Both still reported: one is two hops down, the other is not a part-of child.
    assert_eq!(
        reported(&found, "governs-unreciprocated"),
        ["project_hub", "project_hub"]
    );
}

// --- whose error fails the gate (memview #1047) ------------------------------

/// One memory, optionally stamped with the session that wrote it.
fn stamped(dir: &std::path::Path, name: &str, origin: Option<&str>) -> Corpus {
    std::fs::write(
        dir.join("MEMORY.md"),
        format!("# Memory index\n- [x]({name}.md)\n"),
    )
    .expect("write index");
    let origin = origin
        .map(|s| format!("  originSessionId: {s}\n"))
        .unwrap_or_default();
    std::fs::write(
        dir.join(format!("{name}.md")),
        format!(
            "---\nname: {name}\ndescription: d\nmetadata:\n  type: project\n{origin}---\n\nbody\n"
        ),
    )
    .expect("write memory");
    Corpus::load(dir).expect("loads")
}

fn an_error(memory: &str) -> memview::lint::Finding {
    memview::lint::Finding {
        severity: Severity::Error,
        rule: "missing-modified",
        memory: memory.to_string(),
        detail: "d".to_string(),
    }
}

/// Outside a session — the nightly, which gates the corpus commit — every error
/// still fails. This is what keeps the corpus's own standard where it was.
#[test]
fn without_a_session_any_error_still_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = stamped(dir.path(), "project_a", Some("session-2"));
    assert!(!passed_for_session(&corpus, &[an_error("project_a")], None));
}

/// A session still fails on what it wrote itself.
#[test]
fn a_session_still_fails_on_its_own_memory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = stamped(dir.path(), "project_a", Some("session-1"));
    assert!(!passed_for_session(
        &corpus,
        &[an_error("project_a")],
        Some("session-1")
    ));
}

/// ⚠ The whole point: another session's memory does not fail this one's commit.
#[test]
fn another_sessions_memory_does_not_fail_this_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = stamped(dir.path(), "project_a", Some("session-2"));
    assert!(passed_for_session(
        &corpus,
        &[an_error("project_a")],
        Some("session-1")
    ));
}

/// An unstamped memory belongs to nobody, so it fails no session's gate — the
/// #1047 class itself, routed to the dashboard rather than to a bystander. The
/// nightly above is what still refuses to commit it.
#[test]
fn an_unstamped_memory_fails_no_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = stamped(dir.path(), "project_a", None);
    assert!(passed_for_session(
        &corpus,
        &[an_error("project_a")],
        Some("session-1")
    ));
}

/// The index cannot be attributed and every session reads it, so an error there
/// fails whoever is standing in front of it.
#[test]
fn an_error_in_the_index_still_fails_a_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = stamped(dir.path(), "project_a", Some("session-2"));
    assert!(!passed_for_session(
        &corpus,
        &[an_error("MEMORY.md")],
        Some("session-1")
    ));
}

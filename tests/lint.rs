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

/// Any finding of a rule, whatever its severity — `findings` keeps only errors.
fn of_rule(corpus: &Corpus, rule: &str) -> Vec<String> {
    check(corpus, None)
        .into_iter()
        .filter(|f| f.rule == rule)
        .map(|f| f.detail)
        .collect()
}

/// The Read tool stops at 2,000 lines and says nothing about it, so the warning
/// has to arrive while there is still room to act.
///
/// Both of the corpus's splits were reactive — the roadmap was found at 2,403
/// lines and the log it produced was pushed to 2,233 by appending. A rule that
/// only fires past the limit reports a loss rather than preventing one.
#[test]
fn a_memory_growing_toward_the_read_limit_warns_with_headroom() {
    let dir = tempfile::tempdir().expect("tempdir");
    let long = "a line\n".repeat(1100);
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[("project_hub", long.as_str())],
    );
    let warned = of_rule(&found, "nearing-read-limit");
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].contains("from the limit"), "{warned:?}");
    assert!(of_rule(&found, "past-read-limit").is_empty());
}

/// Past 2,000 lines the tail is genuinely not returned, so this is an error and
/// the warning stands down — one cliff, reported once.
#[test]
fn a_memory_past_the_read_limit_is_an_error_and_not_also_a_warning() {
    let dir = tempfile::tempdir().expect("tempdir");
    let long = "a line\n".repeat(2100);
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[("project_hub", long.as_str())],
    );
    assert_eq!(findings(&found, "past-read-limit").len(), 1);
    assert!(of_rule(&found, "nearing-read-limit").is_empty());
}

/// The median memory is 47 lines; the rule must be silent on all of them.
#[test]
fn an_ordinary_memory_says_nothing_about_its_length() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = "a line\n".repeat(47);
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[("project_hub", body.as_str())],
    );
    assert!(of_rule(&found, "nearing-read-limit").is_empty());
    assert!(of_rule(&found, "past-read-limit").is_empty());
}

/// A corpus where one memory declares a retracted figure.
fn corpus_with_retraction(dir: &std::path::Path, quoting_body: &str) -> Corpus {
    std::fs::write(
        dir.join("MEMORY.md"),
        "# Memory index\n- [d](project_declarer.md)\n- [q](project_quoter.md)\n",
    )
    .expect("write index");
    std::fs::write(
        dir.join("project_declarer.md"),
        "---\nname: project_declarer\ndescription: d\nmetadata:\n  type: project\n  \
         retracts:\n    - \"173/173\"\n---\n\nThe 173/173 figures are retracted.\n",
    )
    .expect("write declarer");
    std::fs::write(
        dir.join("project_quoter.md"),
        format!(
            "---\nname: project_quoter\ndescription: d\nmetadata:\n  type: project\n---\n\n{quoting_body}\n"
        ),
    )
    .expect("write quoter");
    Corpus::load(dir).expect("loads")
}

/// The failure this exists for: `project_health_verified_core_lean` retracted
/// `compare-match 173/173` and went on quoting it about twelve times, including
/// in its own description, defended only by a hand-written banner.
#[test]
fn quoting_a_retracted_figure_without_linking_the_retraction_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus_with_retraction(dir.path(), "The gate reported 173/173 bit-exact.");
    assert_eq!(
        findings(&found, "quotes-a-retracted-figure"),
        ["project_quoter"]
    );
}

/// ⚠ The requirement is a LINK, never a phrasing. A grep for the correction
/// banner in a file that has one returned nothing, because it reads "no 173/173
/// figure IS comparable" rather than "not comparable".
#[test]
fn linking_the_retraction_settles_it_whatever_the_words() {
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus_with_retraction(
        dir.path(),
        "The gate reported 173/173 — see [[project_declarer]], no such figure is comparable.",
    );
    assert!(findings(&found, "quotes-a-retracted-figure").is_empty());
}

/// The memory doing the retracting must be able to state the figure it retracts.
#[test]
fn the_memory_that_retracts_a_figure_may_quote_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus_with_retraction(dir.path(), "Nothing to do with that number.");
    assert!(findings(&found, "quotes-a-retracted-figure").is_empty());
}

/// A memory that never mentions the figure is not asked to link anything.
#[test]
fn a_memory_that_does_not_quote_the_figure_is_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus_with_retraction(dir.path(), "An unrelated claim entirely.");
    assert!(findings(&found, "quotes-a-retracted-figure").is_empty());
}

// ── The injection ceiling, as a check rather than as prose (#822) ────────────

fn all_findings(corpus: &Corpus, rule: &str) -> Vec<memview::lint::Finding> {
    check(corpus, None)
        .into_iter()
        .filter(|f| f.rule == rule)
        .collect()
}

/// ⚠ **The rule exists because prose did not work.** "A new line is paid for by
/// demoting a finished one" has been in `MEMORY.md`'s own header — injected into
/// every session, every turn — for weeks, and the root grew past the ceiling
/// anyway. A budget that is only ever asked for is not a budget.
#[test]
fn an_index_past_the_ceiling_is_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let filler = "x".repeat(memview::lint::INDEX_CEILING + 1);
    let found = all_findings(
        &corpus(
            dir.path(),
            &format!("# Memory index\n- [hub](project_hub.md)\n{filler}\n"),
            &[("project_hub", "body")],
        ),
        "index-over-ceiling",
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].detail.contains("over"), "{:?}", found[0].detail);
}

#[test]
fn an_index_under_the_ceiling_is_silent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let found = all_findings(
        &corpus(
            dir.path(),
            "# Memory index\n- [hub](project_hub.md)\n",
            &[("project_hub", "body")],
        ),
        "index-over-ceiling",
    );
    assert!(found.is_empty(), "{found:?}");
}

/// ⚠ **It must stay a WARNING until the corpus reaches zero, and this test is
/// the thing that says so.** `claude-sync` sets `corpus_ok=false` on any lint
/// ERROR and withholds the entire corpus from its history — so promoting this
/// while the root is over would not tighten the budget, it would stop every
/// memory being committed at all. A gate that can never go green is not a
/// signal; that is how memview's own gate became unpassable (#1062). Promote it
/// in a one-word edit once the root is under, and delete this test in the same
/// edit rather than around it.
#[test]
fn the_ceiling_rule_is_a_warning_while_the_corpus_is_over_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let filler = "x".repeat(memview::lint::INDEX_CEILING + 1);
    let found = all_findings(
        &corpus(
            dir.path(),
            &format!("# Memory index\n- [hub](project_hub.md)\n{filler}\n"),
            &[("project_hub", "body")],
        ),
        "index-over-ceiling",
    );
    assert_eq!(found[0].severity, Severity::Warning);
}

// ── The teaser: a memory's own line in the index (#1310) ─────────────────────
//
// Pippijn, 2026-09-01: "Let's make the teaser text part of the doc itself. The
// automation will be structural, not linguistic." So the field is where the
// index line lives, and the only thing lint can say about it is whether it still
// fits the shape the index needs — one line, and short enough that the ceiling
// can carry a corpus of them.

/// Write a memory whose frontmatter carries `teaser`.
fn corpus_with_teaser(dir: &std::path::Path, teaser: &str) -> Corpus {
    std::fs::write(
        dir.join("MEMORY.md"),
        "# Memory index\n- [t](project_t.md)\n",
    )
    .expect("write index");
    std::fs::write(
        dir.join("project_t.md"),
        format!(
            "---\nname: project_t\ndescription: d\nteaser: {teaser}\nmetadata:\n  type: project\n---\n\nbody\n"
        ),
    )
    .expect("write memory");
    Corpus::load(dir).expect("loads")
}

#[test]
fn a_teaser_the_length_of_an_index_line_is_silent() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The corpus median is 8 bytes; this is longer than that and still fine.
    let found = all_findings(&corpus_with_teaser(dir.path(), "co-use"), "teaser-shape");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_description_pasted_into_the_teaser_is_reported() {
    // The failure this catches: `description` and `teaser` answer different
    // questions — relevance read alone versus a cue read among hundreds — and
    // the wrong one costs the ceiling ~24x per entry.
    let dir = tempfile::tempdir().expect("tempdir");
    let pasted = "x".repeat(memview::lint::TEASER_MAX + 1);
    let found = all_findings(&corpus_with_teaser(dir.path(), &pasted), "teaser-shape");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].detail.contains("over the"),
        "{:?}",
        found[0].detail
    );
}

#[test]
fn a_teaser_at_the_cap_exactly_is_silent() {
    // The boundary, in the direction that can fail: `>` not `>=`.
    let dir = tempfile::tempdir().expect("tempdir");
    let exact = "x".repeat(memview::lint::TEASER_MAX);
    let found = all_findings(&corpus_with_teaser(dir.path(), &exact), "teaser-shape");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_memory_without_a_teaser_is_not_a_finding() {
    // 349 of the corpus lack one. A rule that opens with 349 warnings is a wall
    // nobody works down, and the corpus convention is a warning worked to zero —
    // so absence is COUNTED in the index-stamp, never reported per memory.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = all_findings(
        &corpus(
            dir.path(),
            "# Memory index\n- [hub](project_hub.md)\n",
            &[("project_hub", "body")],
        ),
        "teaser-shape",
    );
    assert!(found.is_empty(), "{found:?}");
}

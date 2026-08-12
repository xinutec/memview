//! The lint pass that leaves the corpus and asks whether what it says is true.
//!
//! Every one of these is a shape that the fifteen intra-corpus rules pass
//! cleanly, because they ask whether the document graph is well-formed and this
//! asks whether it is accurate. The real case behind them: `lares` was retired
//! to `~/Archive/lares` and recorded in one memory while fifteen others still
//! sent a reader to `~/Code/lares`.

use memview::lint::{Severity, check_world};
use memview::store::Corpus;

/// Build a corpus of one memory whose body is `body`, plus a matching index.
fn corpus_saying(dir: &std::path::Path, body: &str) -> Corpus {
    std::fs::write(
        dir.join("MEMORY.md"),
        "# Memory index\n- [p](project_thing.md)\n",
    )
    .expect("write index");
    std::fs::write(
        dir.join("project_thing.md"),
        format!(
            "---\nname: project_thing\ndescription: d\nmetadata:\n  type: project\n---\n\n{body}\n"
        ),
    )
    .expect("write memory");
    Corpus::load(dir).expect("loads")
}

#[test]
fn a_named_repo_that_exists_is_not_a_finding() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(code.path().join("observe")).expect("mkdir");

    let corpus = corpus_saying(corpus_dir.path(), "Captures live at `~/Code/observe/data`.");
    assert!(check_world(&corpus, code.path()).is_empty());
}

#[test]
fn a_named_repo_that_is_gone_is_an_error() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Captures live at `~/Code/lares/captures`.",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "dead-repo-path");
    assert_eq!(findings[0].severity, Severity::Error);
    assert!(findings[0].detail.contains("lares"));
}

#[test]
fn naming_the_archive_location_records_the_retirement_and_clears_it() {
    // The escape hatch is deliberately "say where it went", not "silence this".
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Captures were at `~/Code/lares/captures`; the repo now lives at `~/Archive/lares`.",
    );
    assert!(check_world(&corpus, code.path()).is_empty());
}

#[test]
fn retiring_one_repo_does_not_excuse_a_stale_reference_to_another() {
    // The exemption is per repo, not per document — otherwise a single archive
    // note at the bottom of a long memory would waive every path in it.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "lares moved to `~/Archive/lares`. Deps still come from `~/Code/scanner`.",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("scanner"), "{findings:?}");
}

#[test]
fn a_retirement_note_does_not_reach_a_live_path_further_down_the_document() {
    // The shape the per-document exemption missed, and it is the common one: a
    // long project memory opens with "retired to ~/Archive/lares" and then, forty
    // lines later, still tells a reader "captures live at ~/Code/lares/captures"
    // as a live instruction. Both are the SAME repo, so the per-repo check does
    // not separate them — the banner cleared the whole file.
    //
    // Measured on the real corpus 2026-08-12: `project_lares_recon` carried the
    // retirement banner in its first paragraph and four live paths below it, and
    // `dead-repo-path` was silent on all four.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "lares was retired to `~/Archive/lares`.\n\n\
         Some unrelated paragraph about the port.\n\n\
         Existing captures live at `~/Code/lares/captures` (gitignored, NOT /tmp).\n",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "dead-repo-path");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

#[test]
fn the_absolute_spelling_of_the_same_path_is_read_too() {
    // Both forms occur in the corpus and mean the same place; reading only the
    // tilde form would let the other rot unchecked. The absolute prefix comes
    // from the root under test, which is why no home path appears here.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        &format!("See {}/lares/rust.", code.path().display()),
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

#[test]
fn a_file_inside_a_live_repo_is_not_checked() {
    // Only the repo root is verified. A file inside one moves for ordinary
    // reasons, and reporting those would drown the real signal in churn.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(code.path().join("health")).expect("mkdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Run `~/Code/health/scripts/a-script-that-does-not-exist.sh`.",
    );
    assert!(check_world(&corpus, code.path()).is_empty());
}

#[test]
fn an_unreadable_root_reports_rather_than_passing_silently() {
    // The failure mode this exists to prevent: a check that answers "no
    // findings" because it could not look reads exactly like a clean bill.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_saying(corpus_dir.path(), "Captures at `~/Code/lares`.");

    let findings = check_world(&corpus, std::path::Path::new("/nonexistent/code/root"));

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "unresolvable-code-root");
    assert_eq!(findings[0].severity, Severity::Error);
}

#[test]
fn a_url_that_merely_contains_the_root_path_is_not_a_repo_claim() {
    // The reason this pass parses markdown instead of scanning bytes. A link
    // destination is not a claim about a checkout, and `store.rs` already
    // carries the scar from a line-scanner that could not tell the difference.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        &format!(
            "See [the plan](https://example.invalid{}/lares/plan.md).",
            code.path().display()
        ),
    );
    assert!(
        check_world(&corpus, code.path()).is_empty(),
        "a URL was read as a repo path claim"
    );
}

#[test]
fn a_command_inside_a_fenced_block_is_still_a_claim() {
    // Deliberately the opposite call from the index parser: there a link inside
    // a fence was noise, here a command in a fence is the most actionable thing
    // a memory can say.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Run it:\n\n```sh\n~/Code/lares/deploy.sh\n```\n",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

#[test]
fn a_dead_path_named_only_in_the_description_is_caught() {
    // No markdown node covers frontmatter, so it is appended explicitly — and it
    // matters: the description is the half a reader sees first.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    std::fs::write(
        corpus_dir.path().join("MEMORY.md"),
        "# Memory index\n- [p](project_thing.md)\n",
    )
    .expect("write index");
    std::fs::write(
        corpus_dir.path().join("project_thing.md"),
        "---\nname: project_thing\ndescription: \"captures at ~/Code/lares/captures\"\nmetadata:\n  type: project\n---\n\nNothing in the body.\n",
    )
    .expect("write memory");
    let corpus = Corpus::load(corpus_dir.path()).expect("loads");

    let findings = check_world(&corpus, code.path());
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

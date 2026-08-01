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

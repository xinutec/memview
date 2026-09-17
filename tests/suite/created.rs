//! The lint pass that referees a memory's stated birthday against the
//! transcripts.
//!
//! `created-after-modified` states the one thing decidable inside the corpus: a
//! memory cannot have been changed before it existed. It cannot see the other
//! half — a `created` that is later than the memory's own FIRST WRITE. Such
//! stamps have sat in the corpus unremarked, most of them round-minute values a
//! session typed instead of looking up, and the one the ordering rule did catch
//! was only visible because its `modified` happened to land first.

use memview::lint::{Severity, check_created};
use memview::store::Corpus;

/// A corpus of one memory whose frontmatter declares `created`.
fn corpus_created(dir: &std::path::Path, created: Option<&str>) -> Corpus {
    std::fs::write(
        dir.join("MEMORY.md"),
        "# Memory index\n- [p](project_thing.md)\n",
    )
    .expect("write index");
    let stamp = created.map_or(String::new(), |c| format!("  created: {c}\n"));
    std::fs::write(
        dir.join("project_thing.md"),
        format!(
            "---\nname: project_thing\ndescription: d\nmetadata:\n  type: project\n{stamp}---\n\nbody\n"
        ),
    )
    .expect("write memory");
    Corpus::load(dir).expect("loads")
}

/// Write a `memory-created.json` naming `first` for `project_thing`.
fn record(dir: &std::path::Path, first: &str) -> std::path::PathBuf {
    let at = dir.join("memory-created.json");
    std::fs::write(
        &at,
        format!(r#"{{"project_thing":{{"first":"{first}","session":"s"}}}}"#),
    )
    .expect("write record");
    at
}

#[test]
fn a_birthday_before_the_first_transcript_write_is_not_a_finding() {
    // The expected direction, and why this rule is one-sided. The archive
    // begins 2026-07-31, so a memory older than that shows its first RE-write
    // and not its creation, and reads "early" for exactly this reason.
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_created(dir.path(), Some("2026-08-19T08:34:16.558Z"));
    let at = record(dir.path(), "2026-09-11T04:12:53.633Z");

    assert!(check_created(&corpus, &at).is_empty());
}

#[test]
fn a_birthday_after_the_first_transcript_write_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_created(dir.path(), Some("2026-09-15T15:40:00.000Z"));
    let at = record(dir.path(), "2026-09-15T15:36:15.945Z");

    let findings = check_created(&corpus, &at);

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "created-after-first-write");
    assert_eq!(findings[0].severity, Severity::Error);
    assert_eq!(findings[0].memory, "project_thing");
}

#[test]
fn the_stamping_tools_own_jitter_is_under_the_tolerance() {
    // Measured over the whole corpus: a stamp written by the Write tool lands a
    // hair AFTER the transcript entry for the same write, while the smallest
    // real defect is minutes out — so the two populations are an order of
    // magnitude apart and the
    // tolerance sits in the gap rather than on either edge.
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_created(dir.path(), Some("2026-09-12T15:41:03.247Z"));
    let at = record(dir.path(), "2026-09-12T15:41:50.412Z");

    assert!(check_created(&corpus, &at).is_empty());
}

#[test]
fn a_memory_the_record_does_not_name_is_not_a_finding() {
    // A detection gap, not a memory with no beginning — `memory-dated` draws
    // the same line. Mining misses a write it cannot parse, and inventing a
    // finding from that would accuse the corpus of the miner's blind spot.
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_created(dir.path(), Some("2026-09-15T15:40:00.000Z"));
    let at = dir.path().join("memory-created.json");
    std::fs::write(
        &at,
        r#"{"project_other":{"first":"2026-01-01T00:00:00.000Z"}}"#,
    )
    .expect("write record");

    assert!(check_created(&corpus, &at).is_empty());
}

#[test]
fn a_memory_declaring_no_birthday_is_left_to_the_other_rules() {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_created(dir.path(), None);
    let at = record(dir.path(), "2026-09-15T15:36:15.945Z");

    assert!(check_created(&corpus, &at).is_empty());
}

#[test]
fn an_unreadable_record_reports_rather_than_passing_silently() {
    // Same reason as `unresolvable-code-root`: this rule's whole input is an
    // artefact produced by a separate mining run, so "no findings" and "never
    // ran" are the same output unless one of them says so. A WARNING and not an
    // error, because the record is absent on every machine but the Mac and a
    // lint that cannot run on a fresh checkout is one nobody runs.
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_created(dir.path(), Some("2026-09-15T15:40:00.000Z"));

    let findings = check_created(&corpus, &dir.path().join("absent.json"));

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "unreadable-created-record");
    assert_eq!(findings[0].severity, Severity::Warning);
}

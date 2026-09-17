//! The assembled index, and the four ways it and the written one disagree.

use memview::shadow::{assemble, render};
use memview::store::Corpus;

/// A corpus of an index plus memories, each optionally declaring a teaser.
fn corpus(dir: &std::path::Path, index: &str, docs: &[(&str, Option<&str>)]) -> Corpus {
    std::fs::write(dir.join("MEMORY.md"), index).expect("write index");
    for (name, teaser) in docs {
        let line = teaser.map_or(String::new(), |t| format!("teaser: {t}\n"));
        std::fs::write(
            dir.join(format!("{name}.md")),
            format!(
                "---\nname: {name}\ndescription: d\n{line}metadata:\n  type: project\n---\n\nbody\n"
            ),
        )
        .expect("write memory");
    }
    Corpus::load(dir).expect("loads")
}

/// ⚠ **The finding the artefact exists for**: an index line that has come apart
/// from the memory it describes. Before the teaser lived in the doc, nothing
/// connected the two.
#[test]
fn a_line_that_disagrees_with_its_memory_is_drift() {
    let dir = tempfile::tempdir().expect("tempdir");
    let c = corpus(
        dir.path(),
        "## Rules\n- [stale cue](a.md)\n- [agrees](b.md)\n",
        &[("a", Some("the CURRENT cue")), ("b", Some("agrees"))],
    );
    let s = assemble(&c);
    let drifted: Vec<&str> = s.drifted.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(drifted, vec!["a"]);
    assert_eq!(s.drifted[0].written.as_deref(), Some("stale cue"));
    assert_eq!(s.drifted[0].teaser, "the CURRENT cue");
    // ⚠ And the assembled file carries the MEMORY's text, not the file's — the
    // whole point is that the memory is the authority on its own cue.
    assert!(render(&s).contains("- [the CURRENT cue](a.md)"));
}

/// ⚠ **Two absences, and they are not the same claim.** A memory that declares
/// no teaser cannot be assembled at all, which says nothing about whether it
/// belongs; a memory that declares one and is not carried says only that
/// somebody wrote a line for it. Neither is a proposal, and folding them
/// together would make the artefact read as a rewrite.
#[test]
fn the_two_kinds_of_absence_are_reported_apart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let c = corpus(
        dir.path(),
        "## Rules\n- [carried](a.md)\n- [no teaser here](b.md)\n",
        &[
            ("a", Some("carried")),
            ("b", None),
            ("c", Some("declared, never carried")),
        ],
    );
    let s = assemble(&c);
    assert_eq!(s.no_teaser, vec!["b".to_string()]);
    assert_eq!(s.declared_not_carried, vec!["c".to_string()]);
    // Neither appears in the assembled file: one cannot be spelled, the other
    // has no section to sit under.
    let out = render(&s);
    assert!(!out.contains("(b.md)"), "b must not be invented: {out}");
    assert!(!out.contains("(c.md)"), "c must not be admitted: {out}");
}

/// ⚠ **Order is INHERITED, not decided.** A generated ordering would make every
/// line a diff line and bury the membership findings. So the assembled file
/// follows the written one, which means a matching order is the algorithm
/// declining to have an opinion rather than agreeing.
#[test]
fn the_written_order_is_preserved_within_a_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Alphabetically z, m, a — so any sort would visibly reorder them.
    let c = corpus(
        dir.path(),
        "## Rules\n- [z](z.md)\n- [m](m.md)\n- [a](a.md)\n",
        &[("z", Some("z")), ("m", Some("m")), ("a", Some("a"))],
    );
    let s = assemble(&c);
    let names: Vec<&str> = s.sections[0]
        .lines
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(names, vec!["z", "m", "a"]);
}

/// A link inside a fenced code block is not an index entry — the reason this
/// reads the file with the markdown parser instead of a pattern, recorded in
/// `store::index_entries`.
#[test]
fn a_link_in_a_code_fence_is_not_an_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let c = corpus(
        dir.path(),
        "## Rules\n- [real](a.md)\n\n```\n- [example](b.md)\n```\n",
        &[("a", Some("real")), ("b", Some("b"))],
    );
    let s = assemble(&c);
    // `b` is not indexed, so it is declared-not-carried rather than a line.
    assert_eq!(s.declared_not_carried, vec!["b".to_string()]);
    assert_eq!(s.sections[0].lines.len(), 1);
}

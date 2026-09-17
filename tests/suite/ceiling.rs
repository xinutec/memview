//! What a session is given when the index is over the injection ceiling.

use memview::ceiling::{Cut, INDEX_CEILING, cut};
use memview::store::index_links;

/// An index of `sections` sections, each with a heading and one entry line.
fn index(sections: usize) -> String {
    let mut out = String::from("# Memory index\n\n");
    for n in 0..sections {
        out.push_str(&format!(
            "## section {n} — ⚠ a heading with multi-byte text\n- [entry {n}](memory_{n}.md), [second {n}](other_{n}.md)\n\n"
        ));
    }
    out
}

#[test]
fn a_file_under_the_ceiling_arrives_whole() {
    let md = index(3);
    assert!(md.len() < 1000, "fixture must be well under the ceiling");
    let seen = cut(&md, 1000);
    assert!(seen.is_whole());
    assert_eq!(seen.kept, md);
    assert_eq!(seen.dropped, "");
}

#[test]
fn a_file_exactly_at_the_ceiling_arrives_whole() {
    let md = index(3);
    let seen = cut(&md, md.len());
    assert!(
        seen.is_whole(),
        "the ceiling is a limit, not an exclusive bound"
    );
}

#[test]
fn the_cut_lands_on_a_line_boundary_and_loses_the_tail() {
    let md = index(20);
    let seen = cut(&md, md.len() - 40);
    assert!(!seen.is_whole());
    assert!(
        seen.kept.ends_with('\n'),
        "kept must be whole lines: {:?}",
        &seen.kept[seen.kept.len() - 30..]
    );
    assert!(seen.kept.len() <= md.len() - 40);
    assert_eq!(
        format!("{}{}", seen.kept, seen.dropped),
        md,
        "the halves must rebuild the file"
    );
    // Non-vacuity: a real entry was lost, not just the trailing blank line.
    assert!(
        seen.dropped.contains("[entry 19]"),
        "the last entry should be gone: {:?}",
        seen.dropped
    );
}

#[test]
fn the_dropped_text_names_the_memories_no_session_sees() {
    let md = index(20);
    let seen = cut(&md, md.len() - 40);
    let lost = index_links(seen.dropped);
    assert!(
        !lost.is_empty(),
        "a cut that names no memory would report nothing"
    );
    assert!(lost.contains(&"memory_19".to_string()), "got {lost:?}");
    // And the survivors are not reported as lost.
    assert!(!lost.contains(&"memory_0".to_string()), "got {lost:?}");
}

#[test]
fn a_first_line_over_the_ceiling_drops_everything_rather_than_splitting_it() {
    let md = "a single very long line with no newline before the limit\n";
    let seen = cut(md, 10);
    assert_eq!(seen.kept, "", "no whole line fits");
    assert_eq!(seen.dropped, md);
}

#[test]
fn the_cut_never_splits_a_character() {
    // Every byte length from nothing to the whole file: a panic here is a slice
    // landing inside one of the corpus's many multi-byte marks.
    let md = index(8);
    for ceiling in 0..=md.len() {
        let seen = cut(&md, ceiling);
        assert_eq!(format!("{}{}", seen.kept, seen.dropped), md);
    }
}

#[test]
fn the_ceiling_is_the_one_the_linter_administers() {
    assert_eq!(
        INDEX_CEILING,
        memview::lint::INDEX_CEILING,
        "two ceilings is two answers"
    );
}

#[test]
fn a_cut_is_comparable_so_a_test_can_state_the_whole_answer() {
    let md = "one\ntwo\nthree\n";
    assert_eq!(
        cut(md, 8),
        Cut {
            kept: "one\ntwo\n",
            dropped: "three\n"
        }
    );
}

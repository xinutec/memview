//! Recovering MEMORY.md's membership from a transcript (#1240).
//!
//! ⚠ **Every failure here is SILENT in production.** The output is a list of
//! names that looks plausible whatever it contains, and it is the pre-period of
//! a pre-registered study — a parser that quietly admits one word of prose adds
//! a memory that was never indexed, and one that drops a link removes a demotion
//! that did happen.

use std::collections::BTreeSet;

use memview::index_history::{Readings, day_of, is_the_index, names_in};

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_link_target_is_a_memory_and_the_label_is_not() {
    let index = "- [Xinutec server fleet](project_xinutec_infra.md) — hosts\n\
                 - [Safety first](feedback_safety_first.md) — verify each step\n";
    assert_eq!(
        names_in(index),
        set(&["project_xinutec_infra", "feedback_safety_first"])
    );
}

/// ⚠ **The artefact this replaces holds `file` and `x` as memories.** Both came
/// from matching prose rather than link targets, and both survived into a
/// pre-registered study's pre-period. A label carrying a file name in backticks
/// is the ordinary case in this index, not an edge one.
#[test]
fn prose_that_names_a_file_is_not_an_entry() {
    let index = "- [The gate](project_fleet_check.md) — run `x.md` and `file.md` first\n";
    assert_eq!(names_in(index), set(&["project_fleet_check"]));
}

#[test]
fn a_url_or_a_heading_link_is_not_a_memory() {
    let index = "- [docs](https://example.org/a.md) and [up](#heading) and \
                 [rel](../other/dir.md) and [caps](Project_Thing.md)\n\
                 - [real](project_thing.md)\n";
    assert_eq!(names_in(index), set(&["project_thing"]));
}

#[test]
fn several_entries_on_one_line_are_all_read() {
    // The index packs pointers this way deliberately — one line, many targets.
    let index =
        "- [Kat](reference_kat.md), [cal](reference_public_calendar.md), [pain](user_pain.md)\n";
    assert_eq!(
        names_in(index),
        set(&["reference_kat", "reference_public_calendar", "user_pain"])
    );
}

#[test]
fn only_the_index_itself_counts_as_a_reading_of_it() {
    assert!(is_the_index(
        "/home/example/.claude/projects/p/memory/MEMORY.md"
    ));
    assert!(!is_the_index(
        "/home/example/.claude/projects/p/memory/project_x.md"
    ));
    // A file that merely ends in the same letters is not it.
    assert!(!is_the_index("/home/example/notes/MY-MEMORY.md"));
}

/// ⚠ **Transcripts are walked file by file, so readings arrive out of order.**
/// Picking "whatever landed last" would choose a different winner per day on a
/// different filesystem — a result that changes with directory order and never
/// says so.
#[test]
fn the_days_last_reading_wins_however_the_readings_arrive() {
    let mut r = Readings::default();
    r.absorb("2026-07-04T18:00:00.000Z", set(&["a", "b"]));
    r.absorb("2026-07-04T09:00:00.000Z", set(&["a"]));
    let history = r.history();
    assert_eq!(history["2026-07-04"], set(&["a", "b"]));
}

/// ⚠ **An empty reading is a Read that returned something else** — an error, a
/// truncation — not a day on which the index was empty. Recording it would
/// invent a mass demotion, and the study downstream reads demotions.
#[test]
fn a_reading_with_no_entries_is_not_an_empty_index() {
    let mut r = Readings::default();
    r.absorb("2026-07-04T09:00:00.000Z", set(&["a"]));
    r.absorb("2026-07-04T18:00:00.000Z", BTreeSet::new());
    assert_eq!(r.history()["2026-07-04"], set(&["a"]));
}

#[test]
fn a_day_is_the_date_part_of_the_stamp() {
    assert_eq!(day_of("2026-07-04T18:00:00.000Z"), Some("2026-07-04"));
    assert_eq!(day_of("short"), None);
}

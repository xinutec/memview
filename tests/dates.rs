//! Putting a memory's creation date back into the memory (#1210).

use memview::dates::{created_in, with_created};

fn memory(extra: &str) -> String {
    format!(
        "---\nname: feedback_x\ndescription: A rule.\nmetadata:\n  node_type: memory\n  type: feedback{extra}\n---\n\nBody text.\n"
    )
}

#[test]
fn the_date_lands_under_the_type_line_and_the_body_is_untouched() {
    let out = with_created(&memory(""), "2026-04-12T22:36:22.813Z").expect("write");
    assert!(out.contains("  type: feedback\n  created: 2026-04-12T22:36:22.813Z\n"));
    assert!(out.ends_with("---\n\nBody text.\n"), "{out}");
}

#[test]
fn an_existing_stamp_is_kept_and_the_file_comes_back_unchanged() {
    let already = memory("\n  created: 2026-04-12T22:36:22.813Z");
    let out = with_created(&already, "2026-04-12T22:36:22.813Z").expect("idempotent");
    assert_eq!(out, already);
}

/// ⚠ **The file's own claim outranks a mined one.** A recovered date that
/// disagrees with what the memory says is a conflict to report, not an
/// overwrite — silently correcting it is how a true date gets replaced by a
/// guess, in a field nobody re-reads.
#[test]
fn a_disagreeing_date_is_refused_rather_than_overwritten() {
    let already = memory("\n  created: 2026-04-12T22:36:22.813Z");
    let err = with_created(&already, "2026-07-01T00:00:00Z").expect_err("must refuse");
    assert!(format!("{err}").contains("already says"), "{err}");
}

/// ⚠ A `created:` in the BODY is prose. Matching it would skip a memory that
/// genuinely needs the stamp — the same trap `memory-stamp` hit with `modified:`.
#[test]
fn a_date_in_the_body_is_not_a_stamp() {
    let prose = "---\nname: x\nmetadata:\n  type: feedback\n---\n\n  created: 2020-01-01\n";
    assert_eq!(created_in(prose), None);
    let out = with_created(prose, "2026-04-12T22:36:22.813Z").expect("write");
    assert!(
        out.contains("  type: feedback\n  created: 2026-04-12"),
        "{out}"
    );
}

/// A file with no `metadata:` block is refused rather than reshaped — writing a
/// frontmatter that was not there is a different operation from stamping one.
#[test]
fn a_memory_with_no_metadata_block_is_refused() {
    let bare = "---\nname: x\n---\n\nBody.\n";
    assert!(with_created(bare, "2026-04-12T22:36:22.813Z").is_err());
}

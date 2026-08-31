//! Whose work is in the index — exercised through the public API.

use memview::last_writer::LastWriter;
use memview::staged::foreign;
use reader::effects::{Did, Effect, Effects, Log};
use reader::shell::Reached;

/// The last-writer fold over an effects artefact in which `who` wrote `path` at
/// `minute`.
///
/// ⚠ Built from EFFECT ROWS rather than by hand, so these exercise the real path
/// from evidence to verdict. A fixture that constructed the fold directly would
/// agree with whatever the fold happened to do.
fn wrote(rows: &[(&str, &str, i64, Did)]) -> LastWriter {
    let mut last = LastWriter::default();
    last.absorb(&effects(rows));
    last
}

fn effects(rows: &[(&str, &str, i64, Did)]) -> Effects {
    let mut log = Log::default();
    for (who, path, minute, did) in rows {
        log.push(Effect {
            call: &format!("c{minute}"),
            agent: who,
            minute: *minute,
            did: *did,
            path: Some(path),
            pattern: None,
            host: None,
            command: "sed -i s/a/b/",
            reached: Reached::Always,
        });
    }
    log.finish("2026-08-30T00:00:00Z")
}

#[test]
fn a_path_another_session_wrote_last_is_named() {
    let fx = wrote(&[(
        "hardware",
        "/Users/example/Code/memview/plan/mirror.dhall",
        100,
        Did::Wrote,
    )]);
    let got = foreign(
        &fx,
        "/Users/example/Code/memview",
        &["plan/mirror.dhall".to_string()],
        "memview",
    );
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].who, "hardware");
}

#[test]
fn a_path_i_wrote_last_is_mine() {
    let fx = wrote(&[(
        "memview",
        "/Users/example/Code/memview/src/lib.rs",
        100,
        Did::Wrote,
    )]);
    let got = foreign(
        &fx,
        "/Users/example/Code/memview",
        &["src/lib.rs".to_string()],
        "memview",
    );
    assert!(got.is_empty(), "{got:?}");
}

/// ⚠ **The LAST writer, not any writer.** Two sessions touch one file all the
/// time; what matters is who touched it most recently, or every shared file
/// warns forever.
#[test]
fn the_most_recent_writer_is_the_one_that_counts() {
    let fx = wrote(&[
        (
            "hardware",
            "/Users/example/Code/memview/src/lib.rs",
            100,
            Did::Wrote,
        ),
        (
            "memview",
            "/Users/example/Code/memview/src/lib.rs",
            200,
            Did::Wrote,
        ),
    ]);
    let got = foreign(
        &fx,
        "/Users/example/Code/memview",
        &["src/lib.rs".to_string()],
        "memview",
    );
    assert!(got.is_empty(), "the later write was mine: {got:?}");
}

/// ⚠ **Unknown is not foreign.** A warning that fires on every new file is one
/// people learn to scroll past — which is the failure this check exists to
/// prevent, not to reproduce.
#[test]
fn a_path_nothing_recorded_is_not_reported() {
    let fx = wrote(&[(
        "hardware",
        "/Users/example/Code/memview/src/other.rs",
        100,
        Did::Wrote,
    )]);
    let got = foreign(
        &fx,
        "/Users/example/Code/memview",
        &["src/brand-new.rs".to_string()],
        "memview",
    );
    assert!(got.is_empty(), "{got:?}");
}

/// ⚠ **A read is not a write.** Every session reads everything; only a write
/// says whose work is sitting in the index.
#[test]
fn merely_reading_a_file_does_not_claim_it() {
    let fx = wrote(&[(
        "hardware",
        "/Users/example/Code/memview/src/lib.rs",
        100,
        Did::Read,
    )]);
    let got = foreign(
        &fx,
        "/Users/example/Code/memview",
        &["src/lib.rs".to_string()],
        "memview",
    );
    assert!(got.is_empty(), "a read claimed the file: {got:?}");
}

// ── The fold itself: carried state is the bug family this repo has paid for.

/// ⚠ **A resumed mine absorbs the tail onto what it carried.** The tail alone
/// knows nothing about a path nobody touched today, and answering "nobody wrote
/// it" would clear exactly the files most likely to be somebody else's.
#[test]
fn absorbing_a_tail_keeps_what_the_carried_map_already_knew() {
    let mut last = wrote(&[(
        "hardware",
        "/Users/example/Code/memview/src/old.rs",
        100,
        Did::Wrote,
    )]);
    // A later scan that mentions a different file entirely.
    last.absorb(&effects(&[(
        "memview",
        "/Users/example/Code/memview/src/new.rs",
        200,
        Did::Wrote,
    )]));
    assert_eq!(last.len(), 2, "the tail replaced the carried map: {last:?}");
    assert_eq!(
        last.who_wrote("/Users/example/Code/memview/src/old.rs")
            .map(|w| w.who.as_str()),
        Some("hardware"),
    );
}

/// ⚠ **A later write in the tail must WIN over the carried one**, or the check
/// keeps blaming whoever touched a file first this week.
#[test]
fn a_later_write_in_the_tail_replaces_the_carried_writer() {
    let mut last = wrote(&[(
        "hardware",
        "/Users/example/Code/memview/src/lib.rs",
        100,
        Did::Wrote,
    )]);
    last.absorb(&effects(&[(
        "memview",
        "/Users/example/Code/memview/src/lib.rs",
        200,
        Did::Wrote,
    )]));
    let got = foreign(
        &last,
        "/Users/example/Code/memview",
        &["src/lib.rs".to_string()],
        "memview",
    );
    assert!(got.is_empty(), "the later write was mine: {got:?}");
}

/// ⚠ **An EARLIER row must not overwrite a later one.** Transcripts are re-read
/// on a resume and a scan can legitimately hand back a row already folded in;
/// taking it would walk the answer backwards.
#[test]
fn an_older_write_does_not_displace_a_newer_one() {
    let mut last = wrote(&[(
        "memview",
        "/Users/example/Code/memview/src/lib.rs",
        200,
        Did::Wrote,
    )]);
    last.absorb(&effects(&[(
        "hardware",
        "/Users/example/Code/memview/src/lib.rs",
        100,
        Did::Wrote,
    )]));
    assert_eq!(
        last.who_wrote("/Users/example/Code/memview/src/lib.rs")
            .map(|w| w.who.as_str()),
        Some("memview"),
        "an older row won",
    );
}

/// ⚠ **Absent is not empty.** A missing artefact read as "nobody wrote
/// anything" would report all-clear from no evidence — worse than not running.
#[test]
fn a_missing_artefact_is_none_rather_than_an_empty_map() {
    let dir = tempfile::tempdir().expect("tempdir");
    let got = LastWriter::load(&dir.path().join("absent.json")).expect("load");
    assert!(got.is_none(), "an absent artefact must not read as empty");
}

#[test]
fn what_is_written_is_what_is_read_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = dir.path().join("last-writer.json");
    let last = wrote(&[(
        "hardware",
        "/Users/example/Code/memview/src/lib.rs",
        100,
        Did::Wrote,
    )]);
    last.save(&at).expect("save");
    let back = LastWriter::load(&at).expect("load").expect("present");
    assert_eq!(back, last);
    assert!(!back.is_empty(), "a round trip of nothing proves nothing");
}

/// ⚠ **An empty path is not a path.** The real artefact grew one on its first
/// full mine — harmless, because a lookup is always `repo/path`, but an entry
/// nothing can address is one a reader has to re-explain every time.
#[test]
fn an_empty_path_is_not_recorded() {
    let last = wrote(&[("hardware", "", 100, Did::Wrote)]);
    assert!(last.is_empty(), "{last:?}");
}

/// ⚠ **An all-clear and a broken path shape must not look the same.** Both make
/// `foreign` empty; only one is good news.
#[test]
fn a_resolved_symlink_is_reported_rather_than_read_as_clean() {
    // The shape the record actually holds, plus TWO stragglers under the
    // resolved spelling — which is what the live artefact looks like, and what
    // defeated an existence check.
    let mut rows: Vec<(&str, &str, i64, Did)> = vec![
        (
            "hardware",
            "/Users/example/Code/memview/src/a.rs",
            100,
            Did::Wrote,
        ),
        (
            "hardware",
            "/Users/example/Code/memview/src/b.rs",
            100,
            Did::Wrote,
        ),
        (
            "hardware",
            "/Users/example/Code/memview/src/c.rs",
            100,
            Did::Wrote,
        ),
    ];
    rows.push((
        "hardware",
        "/Volumes/Backup/code/memview/src/z.rs",
        100,
        Did::Wrote,
    ));
    let last = wrote(&rows);

    let shape = memview::staged::wrong_shape(&last, "/Volumes/Backup/code/memview")
        .expect("the lopsided spelling must be reported");
    assert_eq!(
        shape.given, 1,
        "the stragglers are why existence was not the test"
    );
    assert_eq!(shape.better, "/Users/example/Code/memview");
    assert_eq!(shape.better_count, 3);

    // And the spelling the record knows best raises nothing.
    assert_eq!(
        memview::staged::wrong_shape(&last, "/Users/example/Code/memview"),
        None,
    );
}

/// A repository the record has never heard of at all is still reported, not
/// silently cleared.
#[test]
fn a_repo_with_no_entries_at_all_is_reported() {
    let last = wrote(&[(
        "hardware",
        "/Users/example/Code/memview/src/a.rs",
        100,
        Did::Wrote,
    )]);
    assert!(memview::staged::wrong_shape(&last, "/elsewhere/memview").is_some());
}

/// ⚠ A sibling repository whose name merely STARTS the same must not be taken
/// for this one.
#[test]
fn a_sibling_repo_is_not_mistaken_for_this_one() {
    let last = wrote(&[(
        "hardware",
        "/Users/example/Code/memview-web/src/lib.rs",
        100,
        Did::Wrote,
    )]);
    // `memview-web` is a different repository; nothing here is a better
    // spelling of `memview`, so there is nothing to report and also nothing
    // to match.
    assert_eq!(
        memview::staged::wrong_shape(&last, "/Users/example/Code/memview"),
        None
    );
}

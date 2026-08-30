//! Whose work is in the index — exercised through the public API.

use memview::staged::foreign;
use reader::effects::{Did, Effect, Effects, Log};
use reader::shell::Reached;

/// An effects artefact in which `who` wrote `path` at `minute`.
fn wrote(rows: &[(&str, &str, i64, Did)]) -> Effects {
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

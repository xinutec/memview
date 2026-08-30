//! The resume state a mine starts from, exercised through the public API.
//!
//! ⚠ **The two `load` cases are the point.** An absent file and a damaged one
//! must not give the same answer: a damaged one read as "nothing to resume" would
//! resume from empty folds while the watermarks said the corpus had been read,
//! and the artefact would lose everything the carried state held — silently.

use memview::agents::FirstSeen;
use memview::mine::Carried;

fn stamped(sha: &str, when: &str) -> FirstSeen {
    let mut m = FirstSeen::new();
    m.insert(sha.to_string(), (when.to_string(), "builder".to_string()));
    m
}

#[test]
fn a_missing_file_is_a_first_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let got = Carried::load(&dir.path().join("absent.json")).expect("not an error");
    assert_eq!(got, None);
}

/// ⚠ Separated from the case above deliberately — see the module note.
#[test]
fn a_file_that_will_not_parse_is_fatal_rather_than_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mine-resume.json");
    std::fs::write(&path, "{ this is not json").expect("write");
    assert!(Carried::load(&path).is_err());
}

#[test]
fn what_is_saved_is_what_is_loaded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mine-resume.json");

    let mut carried = Carried {
        generated: "2026-08-30T09:00:00Z".to_string(),
        first_seen: stamped("abc1234", "2026-08-01T00:00:00Z"),
        ..Carried::default()
    };
    carried
        .resolved
        .insert("s-1".to_string(), "builder".to_string());
    carried.days.entry("builder".to_string()).or_default();

    carried.save(&path).expect("saves");
    let back = Carried::load(&path).expect("loads").expect("present");
    assert_eq!(back, carried);
}

/// ⚠ The reason `resolved` is carried at all. A session is named in the HEAD of
/// its transcript; a resumed run reads only the tail, so without this surviving
/// the round trip every long-lived agent comes back as a bare uuid.
#[test]
fn a_session_name_survives_the_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mine-resume.json");

    let mut carried = Carried::default();
    carried
        .resolved
        .insert("9f2c-uuid".to_string(), "recall".to_string());
    carried.save(&path).expect("saves");

    let back = Carried::load(&path).expect("loads").expect("present");
    assert_eq!(
        back.resolved.get("9f2c-uuid").map(String::as_str),
        Some("recall")
    );
}

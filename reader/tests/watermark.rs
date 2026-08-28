//! Whether a transcript can be resumed, and the cases where it must not be
//! (#1240).

use std::io::Write;

use reader::watermark::{Drift, drift, observe};

fn write(path: &std::path::Path, text: &str) {
    std::fs::write(path, text).expect("write");
}

fn append(path: &std::path::Path, text: &str) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open");
    f.write_all(text.as_bytes()).expect("append");
}

#[test]
fn a_file_nothing_touched_is_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\ntwo\n");
    let mark = observe(&p).expect("observe");
    assert_eq!(drift(&p, &mark), Drift::Unchanged);
    assert!(drift(&p, &mark).resumable());
}

/// The case the whole design rests on: the CLI appends, so the prefix stands and
/// only the new bytes need reading.
#[test]
fn an_appended_file_is_resumable_and_says_how_much_is_new() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\ntwo\n");
    let mark = observe(&p).expect("observe");
    append(&p, "three\n");
    assert_eq!(drift(&p, &mark), Drift::Grew { by: 6 });
    assert!(drift(&p, &mark).resumable());
}

/// ⚠ **A wrong resume is silent** — it mines from an offset that means something
/// else and reports no error. So every case that is not provably an append must
/// refuse to resume, and re-reading whole is always correct.
#[test]
fn a_rewritten_prefix_refuses_to_resume() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\ntwo\n");
    let mark = observe(&p).expect("observe");
    // Same length, different bytes: an offset check alone would call this fine.
    write(&p, "one\nXwo\n");
    assert_eq!(drift(&p, &mark), Drift::Rewritten);
    assert!(!drift(&p, &mark).resumable());
}

#[test]
fn a_truncated_file_refuses_to_resume() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\ntwo\n");
    let mark = observe(&p).expect("observe");
    write(&p, "one\n");
    assert_eq!(drift(&p, &mark), Drift::Shrank);
    assert!(!drift(&p, &mark).resumable());
}

/// ⚠ A file replaced with different content of the SAME length and then grown
/// still refuses: the fingerprint is taken at the recorded offset, so growth
/// cannot mask a rewrite behind it.
#[test]
fn growth_does_not_hide_a_rewrite_underneath_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\ntwo\n");
    let mark = observe(&p).expect("observe");
    write(&p, "one\nXwo\nthree\n");
    assert_eq!(drift(&p, &mark), Drift::Rewritten);
}

#[test]
fn a_file_that_is_gone_is_unknown_rather_than_resumable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\n");
    let mark = observe(&p).expect("observe");
    std::fs::remove_file(&p).expect("remove");
    assert_eq!(drift(&p, &mark), Drift::Unknown);
    assert!(!drift(&p, &mark).resumable());
}

/// ⚠ **A record written before the fold state existed must still parse.**
/// `transcript-drift.json` is gitignored and rebuildable, but its reader
/// deliberately fails loudly on a file it cannot understand — so a field added
/// without a default would turn every existing corpus into "delete it and start
/// over" rather than into one carried episode fewer.
#[test]
fn a_record_from_before_the_fold_state_still_reads_as_an_offset() {
    let older = r#"{"read_to":128,"tail_sha":"abc"}"#;
    let back: reader::watermark::Resume = serde_json::from_str(older).expect("parse");
    assert_eq!(back.mark.read_to, 128);
    assert_eq!(back.episode, None);
    assert_eq!(back.prompt, None);
}

#[test]
fn a_resume_record_round_trips_the_episode_it_left_open() {
    let mark = reader::watermark::Watermark {
        read_to: 4096,
        tail_sha: "deadbeef".to_string(),
    };
    let held = reader::watermark::Resume {
        mark,
        episode: Some(17),
        prompt: Some("memview".to_string()),
    };
    let text = serde_json::to_string(&held).expect("write");
    assert_eq!(
        serde_json::from_str::<reader::watermark::Resume>(&text).expect("read"),
        held
    );
}

/// The offset and the fold state sit in one object rather than two files: a
/// resume that knows where to start and not what was open is the bug this
/// carries state to avoid.
#[test]
fn the_fold_state_is_written_beside_the_offset_not_under_it() {
    let held = reader::watermark::Resume::fresh(reader::watermark::Watermark {
        read_to: 1,
        tail_sha: "x".to_string(),
    });
    let text = serde_json::to_string(&held).expect("write");
    assert!(text.contains("\"read_to\":1"), "{text}");
    // Nothing open, so nothing written — an absent field and a null one would
    // read the same to serde but not to a person reading the file.
    assert!(!text.contains("episode"), "{text}");
}

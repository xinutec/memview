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

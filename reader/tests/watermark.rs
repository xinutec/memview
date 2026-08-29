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

// ── What a run may do, given what it read last time (#1240) ──────────────────
//
// ⚠ Every test here is about failing CLOSED. A wrong resume produces no error:
// it mines from an offset that means something else, and the artefact becomes
// quietly untrue. So anything not provably an append must return `Full`.

use std::collections::BTreeMap;

use reader::watermark::{Plan, Resume, plan};

fn marked(p: &std::path::Path) -> Resume {
    Resume::fresh(observe(p).expect("observe"))
}

#[test]
fn a_file_that_only_grew_is_resumed_from_its_offset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\ntwo\n");
    let marks = BTreeMap::from([(p.to_string_lossy().into_owned(), marked(&p))]);
    append(&p, "three\n");

    match plan(&marks, std::slice::from_ref(&p)) {
        Plan::Resume { tails, whole, gone } => {
            assert_eq!(tails.len(), 1);
            assert!(whole.is_empty() && gone.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_nothing_touched_is_read_at_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\n");
    let marks = BTreeMap::from([(p.to_string_lossy().into_owned(), marked(&p))]);

    match plan(&marks, std::slice::from_ref(&p)) {
        // Neither a tail nor a whole read: there is nothing new in it.
        Plan::Resume { tails, whole, .. } => assert!(tails.is_empty() && whole.is_empty()),
        other => panic!("{other:?}"),
    }
}

/// ⚠ **ONE unresumable file discards EVERYTHING.** The artefacts carry no
/// per-transcript provenance, so a re-read cannot have its old contribution
/// subtracted — it would be counted from the carried artefact and again from the
/// file. Partial recovery is not available, however tempting.
#[test]
fn one_rewritten_prefix_forces_a_full_re_mine_of_the_whole_corpus() {
    let dir = tempfile::tempdir().expect("tempdir");
    let good = dir.path().join("good.jsonl");
    let bad = dir.path().join("bad.jsonl");
    write(&good, "one\n");
    write(&bad, "one\ntwo\n");
    let marks = BTreeMap::from([
        (good.to_string_lossy().into_owned(), marked(&good)),
        (bad.to_string_lossy().into_owned(), marked(&bad)),
    ]);
    // The prefix moves, which is what a resume may never survive.
    write(&bad, "ONE\nTWO\nthree\n");

    match plan(&marks, &[good, bad]) {
        Plan::Full { because } => assert!(because.contains("rewritten"), "{because}"),
        other => panic!("expected Full, got {other:?}"),
    }
}

#[test]
fn a_file_shorter_than_it_was_forces_a_full_re_mine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\ntwo\nthree\n");
    let marks = BTreeMap::from([(p.to_string_lossy().into_owned(), marked(&p))]);
    write(&p, "one\n");

    match plan(&marks, &[p]) {
        Plan::Full { because } => assert!(because.contains("shorter"), "{because}"),
        other => panic!("expected Full, got {other:?}"),
    }
}

/// A transcript nothing has read before is read whole — and that does NOT
/// invalidate what is carried, because nothing carried mentions it.
#[test]
fn a_new_transcript_is_read_whole_without_discarding_the_rest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let old = dir.path().join("old.jsonl");
    let new = dir.path().join("new.jsonl");
    write(&old, "one\n");
    write(&new, "fresh\n");
    let marks = BTreeMap::from([(old.to_string_lossy().into_owned(), marked(&old))]);

    match plan(&marks, &[old, new.clone()]) {
        Plan::Resume { whole, .. } => {
            assert_eq!(whole, vec![new.to_string_lossy().into_owned()]);
        }
        other => panic!("{other:?}"),
    }
}

/// ⚠ **A vanished transcript is carried, never a reason to re-mine.** Its rows
/// are history; `carry_forward` already treats memory-days this way on purpose.
/// Forcing a full run on one would mean a full run most days — 343 transcripts
/// disappeared in 22 days, nearly all `/private/tmp` scratch — which is the
/// whole saving spent on bookkeeping.
#[test]
fn a_transcript_that_disappeared_is_carried_and_counted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let here = dir.path().join("here.jsonl");
    write(&here, "one\n");
    let vanished = dir.path().join("vanished.jsonl");
    write(&vanished, "gone\n");
    let marks = BTreeMap::from([
        (here.to_string_lossy().into_owned(), marked(&here)),
        (vanished.to_string_lossy().into_owned(), marked(&vanished)),
    ]);
    std::fs::remove_file(&vanished).expect("remove");

    match plan(&marks, &[here]) {
        Plan::Resume { gone, .. } => {
            assert_eq!(gone, vec![vanished.to_string_lossy().into_owned()]);
        }
        other => panic!("{other:?}"),
    }
}

/// The first run has nothing recorded, so everything is new — and that is a
/// resume with a full worklist, not a `Full`, because there is nothing carried
/// to discard.
#[test]
fn the_first_ever_run_reads_everything_as_new() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("t.jsonl");
    write(&p, "one\n");

    match plan(&BTreeMap::new(), &[p]) {
        Plan::Resume { whole, tails, gone } => {
            assert_eq!(whole.len(), 1);
            assert!(tails.is_empty() && gone.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

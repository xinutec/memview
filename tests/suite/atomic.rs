//! A state file is replaced in one step, or not at all.
//!
//! What these defend, and why it is worth a file: `std::fs::write` truncates and
//! then writes, so between those two the file on disk is short. Every reader of
//! the three files that go through `atomic::write` parses JSON, so a reader
//! arriving in that window gets a parse error rather than either version — and a
//! crash in the window leaves it truncated for good. Each caller then degrades
//! QUIETLY: `ShareStore::load` reads an unparseable state file as "no share
//! exists", and `couse.json` / `agents.json` read as absent.
//!
//! Through the public API only, so what is pinned is the behaviour a caller can
//! rely on and not the temp file's spelling. That the temp is a SIBLING of the
//! target is a real promise though — `rename` across filesystems is `EXDEV`, and
//! in the cluster the target is a PVC mount while `/tmp` is the container's own
//! filesystem — so it is pinned here by what the target's directory holds once a
//! write has finished, and once one has failed.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use memview::atomic;

/// Unique per test AND per process. A fixed path under the temp dir is owned by
/// whoever ran first: two `cargo test` invocations at once (a gate run beside an
/// interactive one) then race on `remove_dir_all` + `create_dir_all`, and the
/// loser fails with `AlreadyExists` having tested nothing.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("memview-atomic-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .expect("listing")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn a_replacement_is_visible_whole_or_not_at_all() {
    let dir = scratch("replace");
    let path = dir.join("share-state.json");

    atomic::write(&path, br#"{"token":"one"}"#).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), br#"{"token":"one"}"#);

    atomic::write(&path, br#"{"token":"two"}"#).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        br#"{"token":"two"}"#,
        "the second write must replace the first entirely, not overlay it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Shorter content over longer is where truncate-then-write is most obviously
/// wrong: the tail of the old document survives unless the replacement is whole.
#[test]
fn a_shorter_document_does_not_leave_the_tail_of_the_longer_one() {
    let dir = scratch("shorter");
    let path = dir.join("couse.json");

    atomic::write(&path, br#"[{"name":"a"},{"name":"b"},{"name":"c"}]"#).unwrap();
    atomic::write(&path, b"[]").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"[]");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The sibling promise, from the outside: a finished write leaves the target and
/// nothing else. A temp under `/tmp` would also satisfy this, which is why the
/// failure case below is the other half of the pin.
#[test]
fn a_finished_write_leaves_no_litter_beside_the_target() {
    let dir = scratch("no-litter");
    let path = dir.join("agents.json");

    atomic::write(&path, b"{}").unwrap();

    assert_eq!(
        names_in(&dir),
        vec!["agents.json".to_string()],
        "a leftover temp is how a directory fills with files nobody can explain"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A write that cannot complete must leave the previous version in place — that
/// is the whole point — and must not leave its temp behind either.
///
/// A directory standing where the target should be is the cheapest way to make
/// the RENAME fail while the write to the temp succeeds, which is exactly the
/// half-done state the old code could leave permanently.
#[test]
fn a_write_that_cannot_finish_leaves_the_previous_version_and_no_temp() {
    let dir = scratch("failed");
    let blocked = dir.join("blocked.json");
    std::fs::create_dir(&blocked).unwrap();

    let keep = dir.join("keep.json");
    atomic::write(&keep, br#"{"kept":true}"#).unwrap();

    assert!(
        atomic::write(&blocked, b"[1]").is_err(),
        "a rename onto a directory cannot succeed and must not be reported as if it had"
    );
    assert!(blocked.is_dir(), "the target must be untouched");
    assert_eq!(
        std::fs::read(&keep).unwrap(),
        br#"{"kept":true}"#,
        "an unrelated file must not be disturbed"
    );
    assert_eq!(
        names_in(&dir),
        vec!["blocked.json".to_string(), "keep.json".to_string()],
        "the temp was written beside the target and must be cleaned up when the rename fails"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **The discriminating test, and the reason the three above are not enough.**
///
/// Ablation, run 2026-08-11: with `atomic::write` reverted to a plain
/// `std::fs::write`, every test above still passed. They had to — truncating and
/// rewriting also replaces the content, also handles a shorter document, also
/// leaves no litter, and also fails on a directory. What those tests pin is the
/// OUTCOME, and both implementations reach the same outcome. The difference is
/// only visible in how they get there.
///
/// A rename REPLACES the directory entry, so the target is a different inode
/// afterwards. A truncate-and-write modifies the file in place and keeps it.
/// That is the mechanism, observed directly and without a race.
#[test]
fn a_replacement_is_a_new_file_and_not_the_old_one_rewritten() {
    let dir = scratch("inode");
    let path = dir.join("share-state.json");

    atomic::write(&path, br#"{"token":"one"}"#).unwrap();
    let first = std::fs::metadata(&path).unwrap().ino();

    atomic::write(&path, br#"{"token":"two"}"#).unwrap();
    let second = std::fs::metadata(&path).unwrap().ino();

    assert_ne!(
        first, second,
        "the target was rewritten in place — which means a reader can see it \
         half-written and a crash can leave it that way"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// And the property itself: a concurrent reader never sees a partial document.
///
/// The payload is large enough that a truncate-then-write spends real time in
/// the short state, so this fails against the old implementation rather than
/// merely being able to. It is the one test here that depends on timing, which
/// is why the inode test above carries the deterministic half of the claim.
#[test]
fn a_reader_alongside_a_writer_never_sees_half_a_document() {
    let dir = scratch("concurrent");
    let path = dir.join("couse.json");

    let small = format!("[{}]", vec!["\"a\""; 64].join(","));
    let large = format!("[{}]", vec!["\"bbbbbbbbbbbbbbbb\""; 40_000].join(","));
    atomic::write(&path, large.as_bytes()).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let (path, stop) = (path.clone(), Arc::clone(&stop));
        let (small, large) = (small.clone(), large.clone());
        std::thread::spawn(move || {
            let mut seen = 0u32;
            while !stop.load(Ordering::Relaxed) {
                if let Ok(got) = std::fs::read_to_string(&path) {
                    assert!(
                        got == small || got == large,
                        "read a document that was neither version: {} bytes",
                        got.len()
                    );
                    seen += 1;
                }
            }
            seen
        })
    };

    for i in 0..40 {
        let body = if i % 2 == 0 { &small } else { &large };
        atomic::write(&path, body.as_bytes()).unwrap();
    }
    stop.store(true, Ordering::Relaxed);

    let seen = reader.join().expect("the reader saw a partial document");
    assert!(
        seen > 0,
        "the reader never got to look, so this proved nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

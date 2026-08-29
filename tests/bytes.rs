//! Where a transcript's bytes go (#1199).
//!
//! ⚠ **Every test here is really the same test: the buckets PARTITION.** The
//! ticket this serves exists because `claude_disk.py` charts three lines that do
//! not sum to the total beside them, so a report whose categories overlap or
//! leak would reproduce the defect it was built to fix — and would do it while
//! looking more detailed.

use std::collections::{BTreeMap, HashSet};

use memview::bytes::{Bytes, Kind, absorb};

fn fold(lines: &[&str]) -> Bytes {
    let mut out = Bytes::default();
    let mut seen = HashSet::new();
    let mut calls = BTreeMap::new();
    for line in lines {
        absorb(&mut out, line, line.len() as u64, &mut seen, &mut calls)
            .expect("a Value re-serialises");
    }
    out
}

fn bytes_on_disk(lines: &[&str]) -> u64 {
    lines.iter().map(|l| l.len() as u64).sum()
}

#[test]
fn every_byte_lands_in_exactly_one_bucket() {
    let lines = [
        r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"hello"}]}}"#,
        r#"{"type":"user","uuid":"b","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        r#"{"type":"file-history-snapshot","uuid":"c","snapshot":"xxxx"}"#,
    ];
    let out = fold(&lines);
    assert_eq!(out.total(), bytes_on_disk(&lines));
}

/// ⚠ **A damaged line is still bytes on disk.** Skipping it would keep the
/// report looking tidy while the total quietly stopped matching the file — the
/// exact failure mode this design exists to avoid.
#[test]
fn a_line_that_is_not_json_is_counted_rather_than_skipped() {
    let lines = [
        r#"{"type":"user","uuid":"a","message":{"content":[]}}"#,
        "not json at all",
    ];
    let out = fold(&lines);
    assert_eq!(out.total(), bytes_on_disk(&lines));
    assert_eq!(out.unparseable, "not json at all".len() as u64);
}

/// The dimension nothing had: half the largest transcript is messages it already
/// contains.
#[test]
fn a_message_seen_twice_is_charged_to_the_second_copy() {
    let one =
        r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"text","text":"x"}]}}"#;
    let out = fold(&[one, one]);
    assert_eq!(out.total(), bytes_on_disk(&[one, one]));
    assert_eq!(out.messages, 1, "one distinct message");
    assert!(
        (out.repeat_share() - 0.5).abs() < 0.01,
        "{}",
        out.repeat_share()
    );
}

/// A result names only the call it answers, so the tool has to be carried from
/// the `tool_use` that opened it — otherwise the biggest category in the corpus
/// is a column of `?`.
#[test]
fn a_result_is_attributed_to_the_tool_that_produced_it() {
    let lines = [
        r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#,
        r#"{"type":"user","uuid":"b","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"out"}]}}"#,
    ];
    let out = fold(&lines);
    let result: u64 = out
        .by
        .iter()
        .filter(|((_, k), _)| matches!(k, Kind::ToolResult(t) if t == "Bash"))
        .map(|(_, n)| *n)
        .sum();
    assert!(result > 0, "{:?}", out.by);
}

/// ⚠ A result whose call is in a stretch we have not read is `?`, NOT charged to
/// whichever tool ran last. Guessing would put real bytes under a real name and
/// be indistinguishable from a measurement.
#[test]
fn a_result_with_no_call_in_this_file_is_unattributed_not_guessed() {
    let lines = [
        r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#,
        r#"{"type":"user","uuid":"b","message":{"content":[{"type":"tool_result","tool_use_id":"UNSEEN","content":"out"}]}}"#,
    ];
    let out = fold(&lines);
    let unknown: u64 = out
        .by
        .iter()
        .filter(|((_, k), _)| matches!(k, Kind::ToolResult(t) if t == "?"))
        .map(|(_, n)| *n)
        .sum();
    assert!(unknown > 0, "{:?}", out.by);
}

/// A line type nothing names must show up as itself. Folding it into the
/// envelope would hide a format change as a growing overhead nobody could
/// explain.
#[test]
fn an_unknown_line_type_is_named_rather_than_absorbed() {
    let line = r#"{"type":"brand-new-thing","uuid":"a","payload":"zzz"}"#;
    let out = fold(&[line]);
    assert_eq!(out.total(), line.len() as u64);
    assert!(
        out.by
            .keys()
            .any(|(_, k)| matches!(k, Kind::Other(t) if t == "brand-new-thing")),
        "{:?}",
        out.by
    );
}

/// The envelope is a real cost — uuids, timestamps, parent links — and naming it
/// is what keeps every other number exact.
#[test]
fn the_envelope_absorbs_what_the_parts_do_not_claim() {
    let line = r#"{"type":"assistant","uuid":"a","parentUuid":"z","timestamp":"2026-01-01T00:00:00Z","message":{"content":[{"type":"text","text":"x"}]}}"#;
    let out = fold(&[line]);
    let envelope: u64 = out
        .by
        .iter()
        .filter(|((_, k), _)| matches!(k, Kind::Envelope))
        .map(|(_, n)| *n)
        .sum();
    assert!(envelope > 0);
    assert_eq!(out.total(), line.len() as u64);
}

// ── The WHERE dimension: parts that sum, and concentration (#1199, #1200) ────

#[test]
fn every_top_level_entry_is_weighed_and_a_directory_is_its_contents() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("loose.json"), "12345").expect("write");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    std::fs::write(dir.path().join("sub/a"), "123").expect("write");
    std::fs::write(dir.path().join("sub/b"), "45").expect("write");

    let parts = memview::bytes::top_level(dir.path()).expect("walk");
    let by = |n: &str| parts.iter().find(|p| p.name == n).expect("present").clone();
    assert_eq!(by("loose.json").bytes, 5);
    assert_eq!(by("sub").bytes, 5);
    assert_eq!(by("sub").files, 2);
}

/// ⚠ **`~/.claude` is itself a symlink to an external volume and entries under it
/// point elsewhere.** Following one would count another disk's bytes into this
/// total, and a cycle would never terminate. A link is its own size, which is
/// what the filesystem charges for it.
#[test]
fn a_symlink_is_counted_as_a_link_and_never_followed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let big = dir.path().join("real");
    std::fs::create_dir(&big).expect("mkdir");
    std::fs::write(big.join("payload"), "x".repeat(1000)).expect("write");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&big, dir.path().join("link")).expect("symlink");

    let parts = memview::bytes::top_level(dir.path()).expect("walk");
    let link = parts.iter().find(|p| p.name == "link").expect("present");
    assert!(
        link.bytes < 1000,
        "a link must not carry its target's bytes"
    );
    assert_eq!(link.files, 1);
}

/// The question a cleanup rests on: is the corpus a thousand equal files, or a
/// handful holding nearly all of it?
#[test]
fn concentration_splits_the_largest_n_from_the_rest() {
    let (top, rest, others) = memview::bytes::concentration(vec![100, 50, 5, 3, 2], 2);
    assert_eq!(top, 150);
    assert_eq!(rest, 10);
    assert_eq!(others, 3);
}

#[test]
fn concentration_of_fewer_files_than_asked_for_leaves_no_remainder() {
    let (top, rest, others) = memview::bytes::concentration(vec![7, 3], 5);
    assert_eq!(top, 10);
    assert_eq!(rest, 0);
    assert_eq!(others, 0);
}

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

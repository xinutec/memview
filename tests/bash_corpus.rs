//! Merging the Bash corpus union, exercised through the public API on the row
//! shapes the real corpus actually holds (memview#1130).
//!
//! ⚠ **Paths here are `/home/example`, not this machine's.** The repo is public
//! and the gate's `DL-TEST-REAL-PATH` row refuses a `/Users/…` in a fixture.

use memview::bash_corpus::merge;

fn row(at: Option<&str>, cmd: &str, ran: &str) -> String {
    match at {
        Some(at) => {
            format!(r#"{{"at":"{at}","cmd":"{cmd}","cwd":"/home/example","ran":"{ran}"}}"#)
        }
        None => format!(r#"{{"cmd":"{cmd}","cwd":"/home/example","ran":"{ran}"}}"#),
    }
}

#[test]
fn an_untimestamped_row_collapses_into_its_timestamped_twin() {
    let union = format!(
        "{}\n{}\n",
        row(Some("2026-08-16T13:11:50.403Z"), "ls -la", "ok"),
        row(None, "ls -la", "ok")
    );
    let merged = merge(&union, "");

    assert_eq!(merged.collapsed, 1);
    assert_eq!(merged.rows.len(), 1);
    assert!(
        merged.rows[0].contains("\"at\""),
        "the twin that survives is the informed one"
    );
    // The whole point: rows fell, subjects did not.
    assert_eq!(merged.subjects_before, 1);
    assert_eq!(merged.subjects_after, 1);
    assert!(merged.safe());
}

#[test]
fn two_timestamps_on_one_identity_are_two_days_and_both_survive() {
    // ⚠ Collapsing on `(cmd, cwd)` instead of on identity would delete one of
    // these, and with it the only record that the command ran on both days.
    let union = format!(
        "{}\n{}\n",
        row(Some("2026-08-16T13:11:50.403Z"), "ls -la", "ok"),
        row(Some("2026-08-20T09:02:11.000Z"), "ls -la", "ok")
    );
    let merged = merge(&union, "");

    assert_eq!(merged.collapsed, 0);
    assert_eq!(merged.rows.len(), 2);
    assert_eq!(merged.subjects_after, 1, "one subject, seen twice");
}

#[test]
fn an_untimestamped_row_with_no_twin_is_kept() {
    // The three rows in the real corpus that look like this carry `ran:unknown`
    // where the timestamped copy of the same command says `ok`. Different
    // identity, so nothing vouches for them, so they stay.
    let union = format!(
        "{}\n{}\n",
        row(Some("2026-08-16T13:11:50.403Z"), "pnpm run day-gate", "ok"),
        row(None, "pnpm run day-gate", "unknown")
    );
    let merged = merge(&union, "");

    assert_eq!(merged.collapsed, 0);
    assert_eq!(merged.rows.len(), 2);
}

#[test]
fn tonights_stamp_collapses_a_bare_row_the_union_has_carried() {
    let union = format!("{}\n", row(None, "git status", "ok"));
    let fresh = format!(
        "{}\n",
        row(Some("2026-08-24T00:36:00.000Z"), "git status", "ok")
    );
    let merged = merge(&union, &fresh);

    assert_eq!(merged.collapsed, 1);
    assert_eq!(merged.rows.len(), 1);
    assert!(merged.safe(), "the subject survives in the fresh row");
}

#[test]
fn losing_a_subject_is_refused_rather_than_written() {
    // Nothing in the merge itself can lose a subject; this is the guard's own
    // arithmetic, and the test exists so the refusal path is not first exercised
    // on a night when it matters.
    let union = format!(
        "{}\n",
        row(Some("2026-08-16T13:11:50.403Z"), "rare-command", "ok")
    );
    let merged = merge(&union, "");
    assert!(merged.safe());

    let empty = merge("", "");
    assert_eq!(empty.subjects_before, 0);
    assert!(empty.safe(), "a first run has no floor to fall below");
}

#[test]
fn identical_rows_are_deduplicated_and_the_output_is_byte_sorted() {
    let line = row(Some("2026-08-16T13:11:50.403Z"), "echo hi", "ok");
    let union = format!(
        "{line}\n{line}\n{}\n",
        row(Some("2026-08-16T13:11:50.403Z"), "aaa", "ok")
    );
    let merged = merge(&union, "");

    assert_eq!(merged.rows.len(), 2, "sort -u still dedups an exact repeat");
    let mut sorted = merged.rows.clone();
    sorted.sort();
    assert_eq!(merged.rows, sorted, "the union is read as a sorted file");
}

#[test]
fn a_line_that_is_not_json_is_kept_and_counted() {
    let union = format!(
        "{}\nnot json at all\n",
        row(Some("2026-08-16T13:11:50.403Z"), "ls", "ok")
    );
    let merged = merge(&union, "");

    assert_eq!(merged.unparsed, 1);
    assert_eq!(
        merged.rows.len(),
        2,
        "an archive does not drop what it cannot read"
    );
}

#[test]
fn key_order_does_not_make_two_rows_out_of_one() {
    // An older miner wrote the same fields in a different order. `sort -u` saw
    // two lines; identity is canonical, so this sees one.
    let union = concat!(
        r#"{"at":"2026-08-16T13:11:50.403Z","cmd":"ls","cwd":"/home/example","ran":"ok"}"#,
        "\n",
        r#"{"ran":"ok","cwd":"/home/example","cmd":"ls"}"#,
        "\n"
    );
    let merged = merge(union, "");

    assert_eq!(merged.collapsed, 1);
    assert_eq!(merged.rows.len(), 1);
}

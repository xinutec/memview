//! What a published usage reading means once time has passed.
//!
//! The whole risk in this module is the second half of that sentence. The
//! reading arrives from the home dashboard hours late as a matter of course —
//! it is refreshed by Claude Code's status line, which belongs to a terminal,
//! and the console's own sessions are headless — so "the window this figure
//! described has already turned over" is the ordinary case rather than a corner
//! of it. See `console/src/usage.rs`.

use std::collections::BTreeMap;

use console::session::{Heard, ResetsAt, Seen};
use console::usage::{Published, fresher, merged, reading};

/// The reading exactly as the dashboard served it while this was written.
fn published() -> Published {
    Published {
        host: "mac-mini".into(),
        ts: "2026-08-04T15:20:39.000Z".into(),
        five_hour_pct: 28.0,
        five_hour_resets_at: "2026-08-04T18:10:00.000Z".into(),
        seven_day_pct: 66.0,
        seven_day_resets_at: "2026-08-07T02:00:00.000Z".into(),
    }
}

/// A moment, as milliseconds since the epoch — the unit the reading speaks.
fn at_ms(stamp: &str) -> i64 {
    let when = time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339)
        .expect("the test's own stamp is not RFC 3339");
    (when.unix_timestamp_nanos() / 1_000_000) as i64
}

#[test]
fn a_window_still_open_says_how_long_is_left() {
    let read = reading(&published(), at_ms("2026-08-04T17:10:00.000Z"));
    assert_eq!(
        read.five_hour.as_ref().unwrap().resets_in_ms,
        Some(3_600_000)
    );
    assert_eq!(read.five_hour.as_ref().unwrap().pct, 28.0);
}

#[test]
fn a_window_that_has_turned_over_reports_no_time_left() {
    // Two hours after the five-hour window reset, which is where this console
    // normally finds itself: 28% was true of a window that no longer exists.
    let read = reading(&published(), at_ms("2026-08-04T20:05:00.000Z"));
    assert_eq!(read.five_hour.as_ref().unwrap().resets_in_ms, None);
    // And the longer window, still the one the reading was taken in, is
    // untouched by its neighbour having expired: 2 days, 5 hours, 55 minutes.
    assert_eq!(
        read.seven_day.as_ref().unwrap().resets_in_ms,
        Some(194_100_000)
    );
}

#[test]
fn the_instant_a_window_turns_over_is_already_past() {
    // The boundary, spelled out because "not in the future" and "in the past"
    // differ by exactly this case and a reading is worth nothing at it.
    let read = reading(&published(), at_ms("2026-08-04T18:10:00.000Z"));
    assert_eq!(read.five_hour.as_ref().unwrap().resets_in_ms, None);
}

#[test]
fn the_age_of_the_reading_travels_with_it() {
    let read = reading(&published(), at_ms("2026-08-04T16:20:39.000Z"));
    assert_eq!(read.age_ms, 3_600_000);
    assert_eq!(read.host, "mac-mini");
}

#[test]
fn a_stamp_that_cannot_be_read_is_no_time_rather_than_the_epoch() {
    let mut broken = published();
    broken.ts = "the day before yesterday".into();
    broken.five_hour_resets_at = "soon".into();
    let read = reading(&broken, at_ms("2026-08-04T17:10:00.000Z"));
    // Not 56 years, which is what the epoch would have made of it.
    assert_eq!(read.age_ms, 0);
    assert_eq!(read.five_hour.as_ref().unwrap().resets_in_ms, None);
}

/// What a session heard on its own stream: a fraction, and an epoch **second**.
fn heard(utilization: f64, resets_at: &str, at: &str) -> Seen {
    Seen {
        utilization,
        resets_at: Some(ResetsAt(at_ms(resets_at) / 1000)),
        at: Heard(at_ms(at)),
    }
}

fn seen(windows: &[(&str, Seen)]) -> BTreeMap<String, Seen> {
    windows
        .iter()
        .map(|(name, it)| ((*name).to_string(), it.clone()))
        .collect()
}

#[test]
fn what_a_session_heard_beats_the_dashboard() {
    // The whole point of reading it off the stream: the dashboard's copy is
    // hours old and its window has turned over, while the console heard the
    // truth a minute ago.
    let now = at_ms("2026-08-04T20:05:00.000Z");
    let live = seen(&[(
        "five_hour",
        heard(0.31, "2026-08-04T23:00:00.000Z", "2026-08-04T20:04:00.000Z"),
    )]);
    let read = merged(&live, Some(&published()), now).expect("a reading");
    assert_eq!(read.five_hour.as_ref().unwrap().pct, 31.0);
    assert_eq!(
        read.five_hour.as_ref().unwrap().resets_in_ms,
        Some(10_500_000)
    );
    // A minute old, not five hours.
    assert_eq!(read.age_ms, 60_000);
    assert_eq!(read.host, "this console");
}

#[test]
fn a_window_nothing_has_reported_falls_back_to_the_dashboard() {
    // One event names one window, so the weekly figure can still be the
    // dashboard's while the five-hour one is first-hand.
    let now = at_ms("2026-08-04T17:10:00.000Z");
    let live = seen(&[(
        "five_hour",
        heard(0.31, "2026-08-04T23:00:00.000Z", "2026-08-04T17:09:00.000Z"),
    )]);
    let read = merged(&live, Some(&published()), now).expect("a reading");
    assert_eq!(read.five_hour.as_ref().unwrap().pct, 31.0);
    assert_eq!(read.seven_day.as_ref().unwrap().pct, 66.0);
}

#[test]
fn a_reset_time_is_seconds_and_is_not_read_as_milliseconds() {
    // ⚠ The CLI's `resetsAt` is an epoch **second** while everything else here
    // is milliseconds. Read as milliseconds it lands in January 1970, every
    // window looks long since turned over, and the console silently shows
    // "reset since" for a figure it heard moments ago.
    let now = at_ms("2026-08-04T20:05:00.000Z");
    let live = seen(&[(
        "seven_day",
        heard(0.66, "2026-08-07T02:00:00.000Z", "2026-08-04T20:00:00.000Z"),
    )]);
    let read = merged(&live, None, now).expect("a reading");
    assert_eq!(
        read.seven_day.as_ref().unwrap().resets_in_ms,
        Some(194_100_000)
    );
}

#[test]
fn nothing_heard_and_no_dashboard_is_no_reading_at_all() {
    // Rather than a strip of zeroes, which would be a claim about the account.
    assert!(merged(&BTreeMap::new(), None, at_ms("2026-08-04T20:05:00.000Z")).is_none());
}

/// One session's copy of a window's figure, as its process last saw it.
fn held(utilization: f64, resets_at: i64, at: i64) -> Seen {
    Seen {
        utilization,
        resets_at: Some(ResetsAt(resets_at)),
        at: Heard(at),
    }
}

/// When both windows turn over. Same instance for every session, since the
/// figure is account-wide.
const TURNS: i64 = 1_786_068_000;

#[test]
fn the_higher_reading_of_one_window_is_the_later_one() {
    // ⚠ **The defect this exists for.** Every session answers `get_usage` from
    // its own process's cached rate-limit headers, so an idle one truthfully
    // reports an hour ago — and reports it *now*. Taking the newest arrival made
    // that stale answer authoritative: measured on the phone as the week's
    // figure flipping 81 → 77 → 81 → 77 on the console's sixty-second beat.
    //
    // Utilisation only rises inside a window, so of two readings of the same
    // window the higher is the later, whoever heard it and whenever it landed.
    let current = held(0.81, TURNS, 1_000);
    let stale = held(0.77, TURNS, 9_000);

    assert!(
        !fresher(&current, &stale),
        "an hour-old figure that arrived a moment ago is still an hour old"
    );
    assert!(fresher(&stale, &current), "and the higher one displaces it");
}

#[test]
fn a_window_that_has_turned_over_beats_the_old_one_outright() {
    // The rule above cannot be "higher wins" alone: a fresh window starts near
    // zero, and the old window's high-water mark would outrank it for a week.
    // A later reset time is a different window instance, and it wins whatever it
    // reads.
    let spent = held(0.94, TURNS, 1_000);
    let fresh = held(0.03, TURNS + 604_800, 2_000);

    assert!(fresher(&spent, &fresh));
    assert!(!fresher(&fresh, &spent), "and the old one cannot come back");
}

#[test]
fn a_reading_with_no_reset_time_falls_back_to_when_it_arrived() {
    // What a `rate_limit_event` carries when the CLI declines to say. There is
    // nothing to compare but arrival, which is the old rule — kept for exactly
    // the case that has no better answer.
    let first = Seen {
        utilization: 0.5,
        resets_at: None,
        at: Heard(1_000),
    };
    let later = Seen {
        utilization: 0.4,
        resets_at: None,
        at: Heard(2_000),
    };

    assert!(fresher(&first, &later));
    assert!(!fresher(&later, &first));
}

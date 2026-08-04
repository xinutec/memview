//! What a published usage reading means once time has passed.
//!
//! The whole risk in this module is the second half of that sentence. The
//! reading arrives from the home dashboard hours late as a matter of course —
//! it is refreshed by Claude Code's status line, which belongs to a terminal,
//! and the console's own sessions are headless — so "the window this figure
//! described has already turned over" is the ordinary case rather than a corner
//! of it. See `console/src/usage.rs`.

use console::usage::{Published, reading};

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
fn at(stamp: &str) -> i64 {
    let when = time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339)
        .expect("the test's own stamp is not RFC 3339");
    (when.unix_timestamp_nanos() / 1_000_000) as i64
}

#[test]
fn a_window_still_open_says_how_long_is_left() {
    let read = reading(&published(), at("2026-08-04T17:10:00.000Z"));
    assert_eq!(read.five_hour.resets_in_ms, Some(3_600_000));
    assert_eq!(read.five_hour.pct, 28.0);
}

#[test]
fn a_window_that_has_turned_over_reports_no_time_left() {
    // Two hours after the five-hour window reset, which is where this console
    // normally finds itself: 28% was true of a window that no longer exists.
    let read = reading(&published(), at("2026-08-04T20:05:00.000Z"));
    assert_eq!(read.five_hour.resets_in_ms, None);
    // And the longer window, still the one the reading was taken in, is
    // untouched by its neighbour having expired: 2 days, 5 hours, 55 minutes.
    assert_eq!(read.seven_day.resets_in_ms, Some(194_100_000));
}

#[test]
fn the_instant_a_window_turns_over_is_already_past() {
    // The boundary, spelled out because "not in the future" and "in the past"
    // differ by exactly this case and a reading is worth nothing at it.
    let read = reading(&published(), at("2026-08-04T18:10:00.000Z"));
    assert_eq!(read.five_hour.resets_in_ms, None);
}

#[test]
fn the_age_of_the_reading_travels_with_it() {
    let read = reading(&published(), at("2026-08-04T16:20:39.000Z"));
    assert_eq!(read.age_ms, 3_600_000);
    assert_eq!(read.host, "mac-mini");
}

#[test]
fn a_stamp_that_cannot_be_read_is_no_time_rather_than_the_epoch() {
    let mut broken = published();
    broken.ts = "the day before yesterday".into();
    broken.five_hour_resets_at = "soon".into();
    let read = reading(&broken, at("2026-08-04T17:10:00.000Z"));
    // Not 56 years, which is what the epoch would have made of it.
    assert_eq!(read.age_ms, 0);
    assert_eq!(read.five_hour.resets_in_ms, None);
}

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
use console::usage::{Published, fresher, here, merged, reading, short_name};

/// The reading exactly as the dashboard served it while this was written.
fn published() -> Published {
    Published {
        host: "mac-mini".into(),
        ts: "2026-08-04T15:20:39.000Z".into(),
        five_hour_pct: 28.0,
        five_hour_resets_at: "2026-08-04T18:10:00.000Z".into(),
        seven_day_pct: 66.0,
        seven_day_resets_at: "2026-08-07T02:00:00.000Z".into(),
        // As the timer pusher claims it since home v9: a live probe's reading.
        measured: true,
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
        // A stream reading is a `rate_limit_event`: the API's own headers,
        // dated. `held()` below is the `get_usage` echo, and stays unmeasured.
        measured: true,
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
    // Attributed to the machine that heard it rather than to the dashboard's
    // `host` field. Compared against `here()` rather than a literal, since the
    // name is whatever box the suite runs on — and NOT asserted to differ from
    // the dashboard's, because the machine publishing to home is usually the one
    // running the console, so the two names coinciding is ordinary.
    assert_eq!(read.host, here());
}

#[test]
fn a_search_domain_does_not_open_a_second_row_for_the_same_machine() {
    // home's claude_usage is PRIMARY KEY (host); mac-mini and mac-mini.local are
    // one machine, and keying on the long form would quietly give it two rows.
    assert_eq!(short_name("mac-mini.local"), "mac-mini");
    assert_eq!(short_name("mac-mini"), "mac-mini");
}

#[test]
fn a_machine_that_will_not_say_its_name_is_named_as_such() {
    // Not empty: a blank provenance reads as "no machine" rather than "this
    // machine would not say", and those want different reactions.
    assert_eq!(short_name(""), "unknown host");
    assert_eq!(short_name(".local"), "unknown host");
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
fn a_models_own_window_is_reported_beside_the_plans() {
    // The Fable scope arrives in the same `get_usage` reply as the two plan
    // windows, so it is exactly as fresh as they are — and it is named by the
    // model rather than by a key, because that is what the CLI now sends.
    let now = at_ms("2026-08-12T16:00:00.000Z");
    let live = seen(&[
        (
            "five_hour",
            heard(0.62, "2026-08-12T18:20:00.000Z", "2026-08-12T15:59:00.000Z"),
        ),
        (
            "model:Fable",
            heard(0.06, "2026-08-14T02:00:00.000Z", "2026-08-12T15:59:00.000Z"),
        ),
    ]);
    let read = merged(&live, None, now).expect("a reading");
    assert_eq!(read.models.len(), 1);
    assert_eq!(read.models[0].model, "Fable");
    assert_eq!(read.models[0].window.pct, 6.0);
    // 34 hours, counted from `now` rather than from when it was heard.
    assert_eq!(read.models[0].window.resets_in_ms, Some(122_400_000));
    // And it does not leak into the plan-wide windows, which is what a bare
    // model name in the same map would have done.
    assert!(read.seven_day.is_none());
}

#[test]
fn the_dashboards_own_reading_carries_no_model_windows() {
    // home's per-model figures come FROM this console, so reading them back
    // would be the console quoting itself and calling it corroboration.
    let now = at_ms("2026-08-04T20:05:00.000Z");
    let read = merged(&BTreeMap::new(), Some(&published()), now).expect("a reading");
    assert!(read.models.is_empty());
}

#[test]
fn nothing_heard_and_no_dashboard_is_no_reading_at_all() {
    // Rather than a strip of zeroes, which would be a claim about the account.
    assert!(merged(&BTreeMap::new(), None, at_ms("2026-08-04T20:05:00.000Z")).is_none());
}

/// One session's copy of a window's figure, as its process last saw it: a
/// `get_usage` echo, cached headers of unknowable age — the weaker kind.
fn held(utilization: f64, resets_at: i64, at: i64) -> Seen {
    Seen {
        utilization,
        resets_at: Some(ResetsAt(resets_at)),
        at: Heard(at),
        measured: false,
    }
}

/// The same window figure, but a MEASUREMENT — the API's own word at a datable
/// instant, as a `rate_limit_event` carries.
fn held_measured(utilization: f64, resets_at: i64, at: i64) -> Seen {
    Seen {
        measured: true,
        ..held(utilization, resets_at, at)
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
fn a_window_end_that_wobbles_by_a_second_is_still_the_same_window() {
    // ⚠ **The defect this exists for (#814), measured 2026-08-12.** The console
    // drew 5h 27% at an age of 31 minutes while the CLI answered 28% for the same
    // window, and no reading had been accepted for half an hour: five samples 45 s
    // apart across three sixty-second `ask_usage` ticks showed the age climbing
    // 1:1 with the clock — 1570 s, 1615 s, 1660 s, 1705 s, 1750 s.
    //
    // The reset instant is not a constant. Two `get_usage` probes 30 s apart both
    // answered `23:19:59.838278`; one ten minutes earlier answered
    // `23:19:59.955616`, and the figure the console was holding said `23:20:01`.
    // It drifts, and not in one direction. Judging *any* difference to be a
    // different window instance meant the highest instant ever heard latched the
    // window shut: every later reading looked like an older instance and was
    // dropped whole — the figure and its arrival time together — until the window
    // really did turn over, hours later.
    //
    // A turnover moves the instant by the length of the window. A second is not a
    // turnover of anything.
    let held_late = held(0.27, TURNS + 2, 1_000);
    let current = held(0.28, TURNS, 61_000);
    assert!(
        fresher(&held_late, &current),
        "a second's disagreement about when the window ends is not a new window"
    );

    // And with nothing to choose between the figures, the same window heard again
    // still refreshes its age — which is the half that was making a good number
    // look half an hour old.
    let same = held(0.93, TURNS + 2, 1_000);
    let again = held(0.93, TURNS - 1, 61_000);
    assert!(fresher(&same, &again));

    // The tolerance must not swallow a real turnover, and the shortest window
    // there is runs five hours.
    let spent = held(0.94, TURNS, 1_000);
    let turned = held(0.03, TURNS + 5 * 60 * 60, 2_000);
    assert!(fresher(&spent, &turned), "five hours on is a new window");
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
        measured: false,
    };
    let later = Seen {
        utilization: 0.4,
        resets_at: None,
        at: Heard(2_000),
        measured: false,
    };

    assert!(fresher(&first, &later));
    assert!(!fresher(&later, &first));
}

#[test]
fn a_confirmation_of_the_same_figure_still_refreshes_its_age() {
    // ⚠ **The half of #113 that made a good number look untrustworthy.** With
    // `candidate.utilization > held.utilization` alone, a session reconfirming
    // the figure already held was discarded — and its arrival time with it — so
    // `at` recorded when the number last WENT UP rather than when it was last
    // heard. A figure confirmed a minute ago was then drawn as an hour old.
    let held_now = held(0.13, TURNS, 1_000);
    let again = held(0.13, TURNS, 61_000);
    assert!(
        fresher(&held_now, &again),
        "the same figure heard again is the same figure, heard again"
    );
    // And the direction still holds: an OLDER arrival saying the same thing
    // teaches nothing and must not displace what is held.
    assert!(!fresher(&again, &held_now));
}

#[test]
fn a_fresher_dashboard_beats_a_stale_live_reading_of_the_same_window() {
    // ⚠ **Measured live 2026-08-07 19:57Z.** The console drew 5h 13% at an age
    // of 55 minutes while the dashboard — six minutes old, same window instance
    // — said 22%. `live(…).or_else(|| published…)` reached for the dashboard
    // only when the live figure was ABSENT, and absent is not the same as older,
    // so the console preferred the worse number because it was its own.
    let now = at_ms("2026-08-04T17:10:00.000Z");
    // Same window instance as `published()`, whose stamp is 15:20:39 — and this
    // live reading was captured an hour before that, at 14:20:39. Both are
    // measurements now, so the later CAPTURE wins, and the dashboard's is later.
    // ⚠ The figures are the tell that this is right: the older live reading is
    // LOWER (13%), the newer dashboard HIGHER (28%), because utilisation rises
    // through a window — so believing the later capture also happens to believe
    // the larger figure here. The next test breaks that coincidence apart.
    let live = seen(&[(
        "five_hour",
        heard(0.13, "2026-08-04T18:10:00.000Z", "2026-08-04T14:20:39.000Z"),
    )]);
    let read = merged(&live, Some(&published()), now).expect("a reading");
    // Within a whisker rather than exactly: judging the two sources against each
    // other means putting the dashboard's percentage into the fraction the live
    // side speaks, and 28 → 0.28 → 28.000000000000004 does not survive the trip.
    // The live path has always done this (the running console serves
    // `14.000000000000002`) and the strip rounds — `Math.round(window.pct)` in
    // `usage-strip.ts` — so nothing on screen moves.
    let five = read.five_hour.as_ref().unwrap().pct;
    assert!(
        (five - 28.0).abs() < 1e-9,
        "the dashboard's higher figure of the same window is the later one, got {five}"
    );
    // And the age is the dashboard's, not this console's — a borrowed figure
    // shown with a local age is the same lie pointing the other way.
    assert_eq!(
        read.age_ms,
        at_ms("2026-08-04T17:10:00.000Z") - at_ms("2026-08-04T15:20:39.000Z")
    );
    assert_eq!(read.host, "mac-mini");
}

#[test]
fn a_live_reading_still_wins_when_it_is_the_higher_one() {
    // The fix must not swing the other way: within one window instance the
    // console's own figure wins whenever it is the greater, however old the
    // dashboard's stamp.
    let now = at_ms("2026-08-04T17:10:00.000Z");
    let live = seen(&[(
        "five_hour",
        heard(0.44, "2026-08-04T18:10:00.000Z", "2026-08-04T16:10:00.000Z"),
    )]);
    let read = merged(&live, Some(&published()), now).expect("a reading");
    assert_eq!(read.five_hour.as_ref().unwrap().pct, 44.0);
    assert_eq!(read.host, here());
}

#[test]
fn an_unmeasured_dashboard_row_may_fill_in_but_never_lower() {
    // The row's own claim decides now (home v9): until 2026-09-02 every
    // dashboard row was branded a measurement on arrival, so any writer's
    // cache — the statusLine stamps cached headers with the SEND time — could
    // arrive as dated truth entitled to lower a figure. An echo row keeps the
    // echo's rights: it supplies a window nothing live has reported, and it
    // does not outrank a held measurement, whatever its stamp says.
    let now = at_ms("2026-08-04T17:10:00.000Z");
    let live = seen(&[(
        "five_hour",
        heard(0.62, "2026-08-04T18:10:00.000Z", "2026-08-04T16:40:00.000Z"),
    )]);
    let mut dash = published();
    dash.measured = false;
    dash.ts = "2026-08-04T17:00:00.000Z".into(); // later than the live capture
    dash.five_hour_pct = 11.0; // and lower — the stale-cache shape exactly
    let read = merged(&live, Some(&dash), now).expect("a reading");
    let five = read.five_hour.as_ref().unwrap().pct;
    assert!(
        (five - 62.0).abs() < 1e-9,
        "an echo row lowered a held measurement, got {five}"
    );
    // Filling in is still its job: nothing live spoke to the weekly window, so
    // the echo's copy is better than a blank.
    let seven = read.seven_day.as_ref().unwrap().pct;
    assert!(
        (seven - 66.0).abs() < 1e-9,
        "the echo should fill in, got {seven}"
    );
}

#[test]
fn a_measured_dashboard_row_is_how_another_machines_reset_arrives() {
    // The complement: a row whose writer measured it — the timer pusher's live
    // probe — may move the figure BOTH ways, and down is how a mid-window reset
    // reaches a console none of whose own sessions has spoken since.
    let now = at_ms("2026-08-04T17:10:00.000Z");
    let live = seen(&[(
        "seven_day",
        heard(0.62, "2026-08-07T02:00:00.000Z", "2026-08-04T15:00:00.000Z"),
    )]);
    let mut dash = published();
    dash.seven_day_pct = 11.0; // same window instance, figure fell: a reset
    let read = merged(&live, Some(&dash), now).expect("a reading");
    let seven = read.seven_day.as_ref().unwrap().pct;
    assert!(
        (seven - 11.0).abs() < 1e-9,
        "the later measurement is the account now, got {seven}"
    );
}

#[test]
fn a_reading_outlives_the_session_that_heard_it() {
    // ⚠ **The backwards jump, #87.** Two sessions, one holding 93% and one 92%.
    // The roster used to rebuild this from its live sessions on every poll, so
    // when the 93% session ended the highest remaining was 92% — and the front
    // page, polling every five seconds, showed 92 → 93 → 92 with nothing about
    // the account having changed.
    let mut known = BTreeMap::new();
    console::usage::remember(
        &mut known,
        [("seven_day".to_string(), held(0.93, TURNS, 1_000))],
    );
    // The next poll: that session is gone, and only the lower reading is left.
    console::usage::remember(
        &mut known,
        [("seven_day".to_string(), held(0.92, TURNS, 2_000))],
    );
    assert_eq!(
        known["seven_day"].utilization, 0.93,
        "a window's figure may not fall while the window is the same one"
    );
}

#[test]
fn but_a_window_that_has_turned_over_does_drop_the_old_high_water_mark() {
    // Remembering must not become refusing to let go: the whole point of a reset
    // is that the figure falls, and a later instance displaces the old one
    // however high it was.
    let mut known = BTreeMap::new();
    console::usage::remember(
        &mut known,
        [("five_hour".to_string(), held(0.93, TURNS, 1_000))],
    );
    console::usage::remember(
        &mut known,
        [("five_hour".to_string(), held(0.03, TURNS + 18_000, 2_000))],
    );
    assert_eq!(
        known["five_hour"].utilization, 0.03,
        "a fresh window's honest 3% beats the old window's high-water mark"
    );
}

/// Who gets asked what the account has spent, highest first. See memview #817.
mod who_is_asked {
    use console::usage::asked_before;

    /// The pick a roster would make, without needing live sessions to make it.
    fn asked(sessions: &[(&str, bool, i64)]) -> &'static str {
        let mut best: Option<(&str, (bool, i64))> = None;
        for (name, working, heard) in sessions {
            let rank = asked_before(*working, *heard);
            // `max_by_key`'s rule: a later equal does not displace an earlier.
            if best.is_none_or(|(_, held)| rank > held) {
                best = Some((name, rank));
            }
        }
        // Leaked to keep the helper's signature simple; the names are literals.
        Box::leak(best.expect("nobody to ask").0.to_string().into_boxed_str())
    }

    #[test]
    fn an_idle_session_is_asked_before_a_busier_but_more_recent_one() {
        // ⚠ **The whole of #817.** `busy` spoke most recently and therefore holds
        // the freshest figure — and will not answer until its turn ends, which is
        // how the ages drifted to 109s against a sixty-second beat. `idle` has a
        // cache seconds older and answers now.
        assert_eq!(
            asked(&[("idle", false, 1_000), ("busy", true, 9_999)]),
            "idle"
        );
    }

    #[test]
    fn among_idle_sessions_the_freshest_cache_wins() {
        // Recency still decides, because the answer is only as new as that
        // process's last request — an hour-idle session answers about an hour ago.
        assert_eq!(
            asked(&[("stale", false, 1_000), ("fresh", false, 8_000)]),
            "fresh"
        );
    }

    #[test]
    fn a_fleet_that_is_entirely_busy_still_gets_asked() {
        // Falls back to the old rule rather than to nobody: a deferred answer is
        // worth more than a figure that never updates while everything works.
        assert_eq!(
            asked(&[("older", true, 1_000), ("newer", true, 8_000)]),
            "newer"
        );
    }
}

#[test]
fn a_measured_reset_within_one_window_is_believed() {
    // ⚠ **The defect this exists for, 2026-09-02.** Anthropic reset the week's
    // meter mid-window: the SAME window instance (same `resets_at`) went from a
    // high figure to a low one. The monotone rule refused it — "same window,
    // lower figure" is what a stale echo looks like too — and the console showed
    // the pre-reset number for days. Provenance is the only thing that tells the
    // two apart: a `rate_limit_event` is the API's own word at a known instant,
    // so a later one is the account NOW, downward included.
    let before = held_measured(0.62, TURNS, 1_000);
    let after = held_measured(0.06, TURNS, 2_000);
    assert!(
        fresher(&before, &after),
        "a later measurement of the same window is believed even when it falls"
    );
    // ⚠ Under A the pre-reset high CAN return, because a higher reading is
    // believed as real new usage from any source — the console cannot tell a
    // stale echo of 62% from usage that genuinely climbed back to 62%, and it
    // chose to trust the everyday rise over instantly latching a rare reset.
    // What it must NOT do is let an OLDER low reading undo the reset: an
    // out-of-order measurement stamped before the reset does not win.
    let stale_low = held_measured(0.06, TURNS, 500);
    assert!(
        !fresher(&after, &stale_low),
        "a measurement older than the reset does not lower it further out of order"
    );
}

#[test]
fn an_echoed_drop_within_one_window_is_still_refused() {
    // The other half, and why the fix is provenance and not "believe any drop".
    // A `get_usage` answer is a session's cached headers of unknowable age, so a
    // lower one is the 81 → 77 → 81 flap — refused, exactly as before. Only a
    // dated measurement may lower the figure inside one window instance.
    let cached = held(0.81, TURNS, 1_000);
    let echo = held(0.77, TURNS, 9_000);
    assert!(
        !fresher(&cached, &echo),
        "a cached lower echo does not displace"
    );
    assert!(fresher(&echo, &cached), "the higher echo still does");
}

#[test]
fn a_fresh_higher_echo_beats_a_stale_measurement() {
    // ⚠ **The 2026-09-02 regression this reverses (`03eb36e`), and the reason A
    // exists.** A `get_usage` reply from the session you are talking to is an
    // echo; the home dashboard is a measurement. That commit let a measurement
    // win over an echo UNCONDITIONALLY, so a fresh, higher live reading could
    // not displace the five-minute-old dashboard and the figure stopped
    // tracking your messages. A higher figure is real usage whatever reported
    // it, so the echo wins — this is the assertion that fails if the
    // measurement-beats-echo rule ever comes back.
    let dashboard = held_measured(0.80, TURNS, 1_000);
    let fresh_echo = held(0.84, TURNS, 2_000);
    assert!(
        fresher(&dashboard, &fresh_echo),
        "a fresh higher echo must beat a stale measurement, or the figure stops tracking you"
    );
}

#[test]
fn only_a_measurement_may_lower_within_a_window() {
    // The line A draws (2026-09-02): within one window instance an echo — a
    // `get_usage` reply from a process cache of unknowable age — may RAISE the
    // figure, because usage that rose cannot be faked, but it may NOT lower it,
    // or a stale one would forge a reset. A measurement (a `rate_limit_event`,
    // or a live dashboard probe) may lower, and that is how a real reset lands.
    let base = held(0.30, TURNS, 1_000);
    let echo_up = held(0.50, TURNS, 2_000);
    let echo_down = held(0.06, TURNS, 2_000);
    let measured_down = held_measured(0.06, TURNS, 2_000);
    assert!(
        fresher(&base, &echo_up),
        "an echo may raise: usage that rose is real"
    );
    assert!(
        !fresher(&base, &echo_down),
        "an echo may not lower: that is the flap"
    );
    assert!(
        fresher(&base, &measured_down),
        "a measurement may lower: that is how a reset lands"
    );
}

//! When a reconnecting client may keep the page it is already holding.
//!
//! The rule is small and the consequences of getting it wrong are asymmetric, so
//! it is tested as arithmetic rather than through a session: a wrongly *refused*
//! resume costs a redraw somebody will notice, and a wrongly *granted* one puts a
//! silent hole in a transcript that reads as complete.

use console::session::resumable;

/// A log holding events 40 through 60 — the shape after a session has run on
/// long enough for its scrollback to have dropped the first thirty-nine.
const FROM: u64 = 40;
const ISSUED: u64 = 60;

#[test]
fn a_client_in_the_middle_of_what_is_held_resumes() {
    assert!(resumable(50, FROM, ISSUED));
}

#[test]
fn a_client_that_has_everything_resumes_with_nothing_to_send() {
    // The ordinary reconnect: away for ten seconds, nothing happened meanwhile.
    assert!(resumable(ISSUED, FROM, ISSUED));
}

#[test]
fn a_client_one_short_of_the_front_still_resumes() {
    // It holds 39; the next event it needs is 40, which is the oldest still held.
    // The boundary is the whole reason this is `after + 1` and not `after`.
    assert!(resumable(FROM - 1, FROM, ISSUED));
}

#[test]
fn a_client_that_fell_out_of_the_scrollback_starts_again() {
    // It holds 38, so it needs 39 — dropped. Everything from 40 on would arrive
    // looking continuous, and the missing turn would never be noticed.
    assert!(!resumable(FROM - 2, FROM, ISSUED));
}

#[test]
fn a_client_quoting_a_number_never_issued_starts_again() {
    // The console was restarted and a session resumed under the same id, so the
    // numbering began again. Honouring 200 here would silence the session until
    // it had produced two hundred events.
    assert!(!resumable(200, FROM, ISSUED));
}

#[test]
fn a_caught_up_client_resumes_against_a_session_that_has_said_nothing() {
    // Nothing logged at all: `held_from` is the next number to be issued, and the
    // only client that can be resumed is one holding exactly what was issued.
    assert!(resumable(0, 1, 0), "a fresh session and a fresh client");
    assert!(!resumable(1, 1, 0), "claims an event that does not exist");
}

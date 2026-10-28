//! What a session looks like from outside: the summary a client draws, and the
//! tally carried across an upgrade.

use std::collections::BTreeMap;

use serde::Serialize;

use super::Pending;

/// What a session has counted that the transcript does not record, for an
/// upgrade to hand on. Anything derivable from the file is derived instead.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Tally {
    /// Seconds since the epoch.
    pub started: u64,
    pub model: Option<String>,
    /// See [`Summary::effort`].
    #[serde(default)]
    pub effort: Option<crate::protocol::Effort>,
    pub cost_usd: f64,
    pub window: Option<u64>,
    pub limit: Option<crate::protocol::Allowance>,
    /// See [`Summary::mode`].
    pub mode: Option<crate::modes::Mode>,
    /// See [`Summary::asked`]. Only for a session with no transcript to read it from.
    #[serde(default)]
    pub asked: Option<String>,
    /// See [`Summary::busy`]; never written to the transcript.
    #[serde(default)]
    pub busy: Option<String>,
    /// Questions the session is blocked on: control messages, not transcript lines.
    #[serde(default)]
    pub pending: BTreeMap<String, Pending>,
    /// See [`Summary::background`]. A re-seed sees only the last page's.
    #[serde(default)]
    pub background: BTreeMap<String, crate::protocol::Called>,
    /// The API's last word on each rate-limit window, by the CLI's name (`five_hour`,
    /// …). Account-wide, but arriving per session.
    #[serde(default)]
    pub spent: BTreeMap<String, Seen>,
    /// The exchange count and how far it has read — carried so an upgrade need not
    /// re-read gigabytes. See [`crate::past::counted`].
    #[serde(default)]
    pub counted: crate::past::Counted,
    /// Messages handed to the CLI and not yet read, commands held for the turn's
    /// end, each message's step through the CLI's queue, and the ids it has echoed:
    /// none written anywhere, and all a withdrawal decides by. Carried, or an
    /// upgrade would forget a message still waiting and give its words back while
    /// the CLI still means to read it.
    #[serde(default)]
    pub unread: Vec<super::state::Unread>,
    #[serde(default)]
    pub held: Vec<super::state::Held>,
    #[serde(default)]
    pub steps: BTreeMap<String, (crate::protocol::Lifecycle, i64)>,
    #[serde(default)]
    pub echoed: BTreeMap<String, i64>,
}

/// When a rate-limit window turns over, in epoch seconds — not [`Heard`]'s clock
/// or unit.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ResetsAt(pub i64);

impl ResetsAt {
    /// The same instant in milliseconds.
    pub fn in_ms(self) -> i64 {
        self.0 * 1000
    }
}

/// When this console heard a reading, in epoch milliseconds — arrival, not freshness.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct Heard(pub i64);

/// One window's utilisation, as last reported.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Seen {
    /// A fraction: 0.28 is 28% of the window.
    pub utilization: f64,
    #[serde(default)]
    pub resets_at: Option<ResetsAt>,
    pub at: Heard,
    /// Whether the API said this at a datable moment. An echo can only fill in; a
    /// measurement moves the figure both ways — [`crate::usage::fresher`].
    #[serde(default)]
    pub measured: bool,
}

/// What a client sees of a session without reading its transcript.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Summary {
    pub id: String,
    pub dir: String,
    /// Seconds since the epoch.
    pub started: u64,
    /// When anything last happened, in milliseconds, from the transcript.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub touched: Option<u64>,
    /// The transcript's size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub bytes: Option<u64>,
    pub alive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub model: Option<String>,
    /// The effort the last answer was given, from the transcript.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub effort: Option<crate::protocol::Effort>,
    /// What the CLI last said it was doing, when it is doing anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub busy: Option<String>,
    /// Whether a turn is running, as observed — [`Self::busy`] is announced only on change.
    pub working: bool,
    /// Exchanges since the last compaction. See [`crate::past::counted`].
    pub interactions: u32,
    /// The tokens at API list prices — not money spent, on a subscription.
    pub cost_usd: f64,
    /// The last request's prompt in tokens, cached parts included.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub context: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub window: Option<u64>,
    /// Background calls started and not reported finished. `nohup … &` is invisible.
    #[serde(skip_serializing_if = "none")]
    pub background: usize,
    /// The background calls still running, by name.
    #[serde(default)]
    pub running: Vec<crate::protocol::Called>,
    /// The account's verdict on its rate limit, once it has given one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<crate::protocol::Allowance>,
    /// The first thing this session was asked to do, kept as its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub asked: Option<String>,
    /// What the conversation calls itself — `memview`, `health`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
    /// What other sessions call this one. Set only at spawn, so it can lag a rename
    /// until the next resume.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub peer_name: Option<String>,
    /// What the session may do without asking, as the console set it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mode: Option<crate::modes::Mode>,
    /// Why the last mode change was refused, until the next is asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mode_refused: Option<String>,
    /// How many questions it is blocked on.
    pub waiting: usize,
    /// Messages written and not read back.
    pub unread: usize,
    /// How long it has failed to read them, in seconds, once it counts as deaf.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub deaf: Option<u64>,
    /// When the shared sign-in was refused, in epoch milliseconds, until a request
    /// is answered again. See [`crate::login`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub signed_out: Option<u64>,
    /// Slash commands waiting for the turn to end, oldest first, word for word.
    #[serde(default)]
    pub held: Vec<HeldCommand>,
}

/// A slash command waiting for the turn to end, with the client's id for it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct HeldCommand {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub id: Option<String>,
}

/// Nothing to report, for a count left off the wire when it is zero.
fn none(count: &usize) -> bool {
    *count == 0
}

//! What a session looks like from outside: the summary a client draws, and the
//! tally carried across an upgrade.

use std::collections::BTreeMap;

use serde::Serialize;

use super::Pending;

/// What a session has counted, for an upgrade to hand on. None of this is in the
/// transcript: cost, rate-limit status, window and model arrive on the stream
/// only. Fullness is deliberately absent, since every assistant message records
/// it and a re-seed recovers it — anything derivable from the file is derived.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Tally {
    /// Seconds since the epoch. Carried because `execve` leaves the child's clock alone.
    pub started: u64,
    pub model: Option<String>,
    pub cost_usd: f64,
    pub window: Option<u64>,
    pub limit: Option<crate::protocol::Allowance>,
    /// See [`Summary::mode`]. Carried because the console is the only thing that knows it.
    pub mode: Option<crate::modes::Mode>,
    /// The first thing this session was asked to do — see [`Summary::asked`]. A
    /// fallback: [`crate::past::opening`] reads it from the head of the transcript and
    /// overwrites this, which is used only for a session with no transcript.
    #[serde(default)]
    pub asked: Option<String>,
    /// What the session is doing, if anything. See [`Summary::busy`]. A re-seed cannot
    /// recover this: a status is announced on stdout when it changes and never
    /// written to the transcript.
    #[serde(default)]
    pub busy: Option<String>,
    /// Questions the session is blocked on. A `can_use_tool` request is a control
    /// message, not a transcript line, and the session stays blocked on it across an
    /// upgrade: dropped, the question is orphaned.
    #[serde(default)]
    pub pending: BTreeMap<String, Pending>,
    /// Background tasks still running — see [`Summary::background`]. A re-seed
    /// recovers only the ones started inside the last page, while `execve` leaves
    /// the children running; what is running now is a fact about the present.
    #[serde(default)]
    pub background: BTreeMap<String, crate::protocol::Called>,
    /// What the API last said about each rate-limit window, keyed by the CLI's own
    /// name (`five_hour`, `seven_day`, …). Account-wide, so any session's reading is
    /// the truth for all; kept per session because that is where the stream arrives.
    #[serde(default)]
    pub spent: BTreeMap<String, Seen>,
    /// The exchange count and how far into the transcript it accounts for. Carried
    /// for the cost: an upgrade re-seeds every session at once, and the files reach
    /// gigabytes. See [`crate::past::counted`].
    #[serde(default)]
    pub counted: crate::past::Counted,
}

/// When a rate-limit window turns over, in epoch seconds — the CLI's unit. It
/// names which instance of the window a figure belongs to. A type rather than an
/// `i64` because it sits beside [`Heard`] and the two are neither the same clock
/// nor the same unit; see [`crate::usage::fresher`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ResetsAt(pub i64);

impl ResetsAt {
    /// The same instant in milliseconds. The one place the conversion is written.
    pub fn in_ms(self) -> i64 {
        self.0 * 1000
    }
}

/// When this console heard a reading, in epoch milliseconds, by this machine's
/// clock. Arrival, not freshness: a session answers from cached headers.
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
    /// Whether the API itself said this, at a moment we can date: a
    /// `rate_limit_event` or a dashboard row is a measurement; a `get_usage` answer is
    /// a cache of unknowable age. A measurement moves the figure both ways, an echo
    /// can only fill in — see [`crate::usage::fresher`]. Defaulted false.
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
    /// When anything last happened, in milliseconds, from the transcript — not
    /// `started`, which is when this console picked the process up. Filled by the
    /// roster; see [`crate::past::touched`]. Absent when the transcript cannot be found.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub touched: Option<u64>,
    /// How much the transcript weighs, in bytes. Not [`Self::context`], which is the
    /// last request's prompt in tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub bytes: Option<u64>,
    pub alive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub model: Option<String>,
    /// What the CLI last said it was doing, when it is doing anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub busy: Option<String>,
    /// Whether a turn is running — observed by the runner, not narrated by the CLI.
    /// [`Self::busy`] cannot answer this: a status is announced when it changes, so
    /// a long stretch of one activity leaves nothing standing.
    /// Deliberately not a timeout: a turn can legitimately be quiet for minutes.
    pub working: bool,
    /// How many times someone has spoken to this session since it was last
    /// compacted — exchanges, not messages. See [`crate::past::counted`].
    pub interactions: u32,
    /// What this session's tokens would have cost at API list prices. Not money: the
    /// session runs on the subscription. Shown only when [`Self::limit`] says the
    /// account has stopped being all-you-can-eat.
    pub cost_usd: f64,
    /// How many tokens the last request's prompt came to, and the window it went
    /// into. Per message, not per turn — the result line sums every request the turn
    /// made. Input + cache-creation + cache-read: the cached part is almost all of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub context: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub window: Option<u64>,
    /// How many background tool calls this session has started and not had reported
    /// finished. Only the ones the harness tracks: `nohup … &` is invisible. Counted
    /// by the runner so the list can rank on it without opening anything.
    #[serde(skip_serializing_if = "none")]
    pub background: usize,
    /// Which background calls are still running. `background` is kept beside this:
    /// the list wants a number, the strip wants the name.
    #[serde(default)]
    pub running: Vec<crate::protocol::Called>,
    /// The account's own verdict on its rate limit, when it has given one:
    /// `allowed`, `allowed_warning` or `rejected`. `None` until the
    /// account says something — the reason cost is hidden by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<crate::protocol::Allowance>,
    /// The first thing this session was asked to do, kept as its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub asked: Option<String>,
    /// What the conversation calls itself — `memview`, `health`. Filled by the roster
    /// from the transcript; see [`crate::past::named`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,

    /// What the other sessions on this machine call this one — the name
    /// `ListAgents` prints and `SendMessage` resolves. Not [`Self::name`], which is
    /// the title in the transcript: only `-n` at spawn writes this one, so a
    /// conversation renamed while it runs keeps the name its peers already knew
    /// until it is next resumed. Shown so that gap is visible rather than
    /// discovered by a session reporting that a name does not exist. Filled by the
    /// roster; see [`crate::peers::named`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub peer_name: Option<String>,
    /// What the session may do without asking: `default`, `plan`, `dontAsk`,
    /// `acceptEdits`, `auto`, `bypassPermissions`. What the console set, not what the
    /// transcript says — a resumed session carries the previous session's mode
    /// lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mode: Option<crate::modes::Mode>,
    /// Why the last mode change was refused, in the CLI's own words. Present only
    /// until the next change is asked for: it describes an attempt, not a state.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mode_refused: Option<String>,
    /// How many questions it is blocked on — the one number that means "this
    /// session cannot go on without you".
    pub waiting: usize,
    /// How many messages have been written to this session and not read back.
    pub unread: usize,
    /// How long it has been failing to read them, in seconds — present only when the
    /// console is prepared to call it deaf. See [`crate::session::Session::deaf`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub deaf: Option<u64>,
    /// Slash commands waiting for the turn to end, oldest first — see [`crate::session::state::State::held`].
    /// The words themselves, because the client draws them and cancels by them.
    #[serde(default)]
    pub held: Vec<String>,
}

/// Nothing to report, for a count left off the wire when it is zero.
fn none(count: &usize) -> bool {
    *count == 0
}

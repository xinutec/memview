//! One live Claude Code session: the subprocess, its transcript, its listeners.
//!
//! A session is one long-lived process: with `--input-format stream-json` it
//! serves turn after turn on an open stdin and exits 0 when stdin closes, so
//! closing stdin is the polite way to end one. The id is ours, chosen before the
//! process exists (`--session-id`), so a client can subscribe to a session that
//! is still starting and `--resume` later takes the same id.

#![expect(
    unsafe_code,
    reason = "libc process control (waitpid, kill, raw descriptors) for the sessions this supervises"
)]

use parking_lot::Mutex;
use std::collections::{BTreeMap, VecDeque};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, oneshot};

use crate::protocol::{self, Event};

/// Where a session's instructions go. A trait object because a session is
/// spawned, with the child's own [`ChildStdin`], or adopted across an upgrade,
/// with the same pipe reopened from a raw descriptor.
type Sink = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// The three pipes to a session's process, by number — so they can outlive this
/// image. See [`crate::roster::Roster::handover`].
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Fds {
    pub stdin: std::os::fd::RawFd,
    pub stdout: std::os::fd::RawFd,
    pub stderr: std::os::fd::RawFd,
}

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
    pub limit: Option<String>,
    /// See [`Summary::mode`]. Carried because the console is the only thing that knows it.
    pub mode: Option<String>,
    /// The first thing this session was asked to do — see [`Summary::asked`]. A
    /// fallback: [`crate::past::opening`] reads it from the HEAD of the transcript and
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

/// When a rate-limit window turns over, in epoch SECONDS — the CLI's unit. It
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

/// When this console heard a reading, in epoch MILLISECONDS, by this machine's
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

/// Wait on an adopted child, so the kernel can let go of it when it ends. An
/// adopted session has no [`Child`] to wait on, so without this each one that ends
/// stays `<defunct>`.
///
/// A blocking `waitpid`, one thread per adopted session. NOT `SIGCHLD` =
/// `SIG_IGN`: that reaps every child and takes the exit status [`Session::reap`]
/// reads, so a clean exit would arrive as `code: None` — this file has a test
/// against it. The status is dropped: end-of-file has already declared an
/// adopted session over ([`Session::read_from`]).
fn reap_adopted(pid: u32) {
    tokio::task::spawn_blocking(move || {
        let mut status = 0;
        // SAFETY: `pid` is a child of this process — `execve` does not change parentage
        // — and `waitpid` only reads its exit status.
        if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) } < 0 {
            // ECHILD is the ordinary case for a session already reaped by an earlier image.
            tracing::debug!(
                "adopted {pid} could not be waited on: {}",
                std::io::Error::last_os_error()
            );
        }
    });
}

/// Take a descriptor out of close-on-exec, and make it non-blocking. Rust sets
/// `O_CLOEXEC` on every pipe it creates, so without the first half an upgraded
/// image inherits nothing; the second is tokio's requirement. False when the
/// descriptor is gone.
pub fn keepable(fd: std::os::fd::RawFd) -> bool {
    // SAFETY: fcntl on a descriptor this process owns; both calls only read or
    // set flags and cannot invalidate it.
    unsafe {
        if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
            return false;
        }
        let flags = libc::fcntl(fd, libc::F_GETFL);
        flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    }
}

/// Whether a `ps` listing shows this conversation still being run. Read through
/// [`crate::past::words_of_claude_processes`], so a line that merely mentions
/// the id is not a `claude`.
pub fn names_session(ps_output: &str, id: &str) -> bool {
    crate::past::words_of_claude_processes(ps_output)
        .iter()
        .any(|word| word == id)
}

/// Kill a stopped session's process, after checking it is still that process: a
/// pid is not a handle, this one is up to thirty seconds old, and a late SIGKILL
/// at a reused pid is a fault nothing can trace. Anything unreadable is NOT a
/// kill — leaving a process alive is the recoverable half.
pub fn finish(pid: u32, id: &str) {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
    else {
        tracing::warn!("could not ask ps about {pid}, so {id} was left alone");
        return;
    };
    if !names_session(&String::from_utf8_lossy(&output.stdout), id) {
        // The ordinary case: the session took its stdin closing as the exit it is.
        tracing::info!("{id} had already gone, so pid {pid} was left alone");
        return;
    }
    tracing::info!("{id} outlived its grace period — killing pid {pid}");
    // SAFETY: a kill to a pid this console started and has just confirmed is still
    // running that session; ESRCH is ignored.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// How much transcript one session keeps in memory: a scrollback, not an archive.
const SCROLLBACK: usize = 5000;

/// How many recent tool calls are remembered so a detached one can be named.
/// Small: the `Tool` event is immediately followed by the `ToolResult` that
/// reveals the detach, and this is held for the life of a process that runs for days.
const CALLED_RING: usize = 32;

/// How long a session gets to finish after its stdin closes, before it is killed.
/// Generous: the clean exit is the one that flushes the transcript.
const GRACE: Duration = Duration::from_secs(30);

/// How much of the child's stderr to keep for diagnosis.
const STDERR_KEPT: usize = 4000;

/// How often to re-read the transcript while the child says nothing. The read
/// is incremental, so this is set by how long a wrong number may stay on screen.
const RECOUNT_EVERY: Duration = Duration::from_secs(5);

/// The CLI's own name for the mode a session runs in when nothing is passed.
/// Displayed as *Manual*: it asks before every tool call that needs permission.
pub const DEFAULT_MODE: &str = "default";

/// What a client sees of a session without reading its transcript.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Summary {
    pub id: String,
    pub dir: String,
    /// Seconds since the epoch.
    pub started: u64,
    /// When anything last happened, in MILLISECONDS, from the transcript — not
    /// `started`, which is when this console picked the process up. Filled by the
    /// roster; see [`crate::past::touched`]. Absent when the transcript cannot be found.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub touched: Option<u64>,
    /// How much the transcript weighs, in bytes. Not [`Self::context`], which is the
    /// LAST request's prompt in tokens.
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
    /// [`Self::busy`] cannot answer this: a status is announced when it CHANGES, so
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
    /// into. Per MESSAGE, not per turn — the result line sums every request the turn
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
    /// WHICH background calls are still running. `background` is kept beside this:
    /// the list wants a number, the strip wants the name.
    #[serde(default)]
    pub running: Vec<crate::protocol::Called>,
    /// The account's own verdict on its rate limit, when it has given one:
    /// `allowed`, `allowed_warning` or `rejected`. `None` until the
    /// account says something — the reason cost is hidden by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<String>,
    /// The first thing this session was asked to do, kept as its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub asked: Option<String>,
    /// What the conversation calls itself — `memview`, `health`. Filled by the roster
    /// from the transcript; see [`crate::past::named`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,

    /// What the OTHER sessions on this machine call this one — the name
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
    /// `acceptEdits`, `auto`, `bypassPermissions`. What the console SET, not what the
    /// transcript says — a resumed session carries the previous session's mode
    /// lines. `default` is shown as *Manual*; the client keeps the label table.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mode: Option<String>,
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
    /// How long it has been failing to read them, in SECONDS — present only when the
    /// console is prepared to call it deaf. See [`Session::deaf`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub deaf: Option<u64>,
    /// Slash commands waiting for the turn to end, oldest first — see [`State::held`].
    /// The words themselves, because the client draws them and cancels by them.
    #[serde(default)]
    pub held: Vec<String>,
}

/// An event and its place in the session's order. The number is what makes a
/// dropped connection survivable: a reconnecting client says what it has and is
/// sent what it missed. Sequential from 1, per session, assigned under the lock
/// that appends to the log.
#[derive(Debug, Clone)]
pub struct Stamped {
    pub seq: u64,
    /// When it happened, in milliseconds since the epoch. See [`protocol::Timed`].
    pub at: Option<i64>,
    pub event: Event,
}

/// Nothing to report, for a count left off the wire when it is zero.
fn none(count: &usize) -> bool {
    *count == 0
}

/// Now, in milliseconds since the epoch.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// What a connecting client is owed, and whether what it already has is good.
#[derive(Debug)]
pub struct Backlog {
    /// False means the client's page cannot be kept: it named nothing, or an event
    /// this session no longer holds.
    pub resumed: bool,
    pub events: Vec<Stamped>,
    /// The highest sequence number this client has now been sent. Live events at or
    /// below it must not be sent again.
    pub through: u64,
}

/// Whether a client holding everything through `after` can be sent only what it
/// missed, given a log holding `held_from..=issued`. Older than the log's front
/// means the scrollback dropped the difference; newer than anything issued means
/// another session's numbering. `after + 1` on the left: the NEXT event is what
/// has to be reachable, so an exactly-caught-up client resumes against an empty log.
pub fn resumable(after: u64, held_from: u64, issued: u64) -> bool {
    after + 1 >= held_from && after <= issued
}

/// A question the session is waiting on an answer to. The arguments are kept
/// because an allow has to echo them back, and the tool's name because only one
/// tool's arguments may be edited — see [`protocol::QUESTION_TOOL`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    pub tool: String,
    /// The call being asked about — the `tool_use` id, carried across an upgrade so a
    /// re-seeded question can still say which tool row it belongs to.
    #[serde(default)]
    pub call: Option<String>,
    pub input: serde_json::Value,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// The mutable half of a session, behind one lock.
#[derive(Debug, Default)]
struct State {
    log: VecDeque<Stamped>,
    /// The last sequence number issued. Not the log's length: the log drops its
    /// front, and a reused number would resume a client into the wrong place.
    issued: u64,
    /// Questions the session is blocked on, by control-request id.
    pending: BTreeMap<String, Pending>,
    alive: bool,
    /// When the kill armed by [`Session::stop`] falls due, in epoch milliseconds;
    /// `None` for a session nobody has stopped. Read by [`crate::roster::Roster::handover`].
    stopping: Option<i64>,
    model: Option<String>,
    busy: Option<String>,
    /// See [`Summary::interactions`], and [`crate::past::counted`] for why the byte
    /// offset travels with the number.
    counted: crate::past::Counted,
    /// See [`Summary::mode`]. Written optimistically when the console asks for a
    /// change, and corrected when the CLI answers — [`Session::settle_mode`].
    mode: Option<String>,
    /// What the mode was before a change the CLI has not answered yet, so a refusal
    /// can put it back. Cleared either way when the answer arrives.
    restore: Option<String>,
    /// See [`Summary::mode_refused`].
    mode_refused: Option<String>,
    cost_usd: f64,
    /// The last turn's prompt size and the window it went into. See
    /// [`Summary::context`].
    context: Option<u64>,
    window: Option<u64>,
    /// See [`Summary::limit`].
    limit: Option<String>,
    /// What the API last said about each rate-limit window. See [`Tally::spent`].
    spent: BTreeMap<String, Seen>,
    /// Background tool calls started and not yet ended, keyed by the call (what a
    /// notification names), carrying the task (what a kill names).
    background: std::collections::BTreeMap<String, crate::protocol::Called>,
    /// The last few tool calls seen, by call id, so a background one can be NAMED
    /// when its result arrives. A ring: an unbounded map would hold every call of a
    /// session that runs for days.
    called: std::collections::VecDeque<(String, crate::protocol::Called)>,
    /// When the process last wrote a line, in epoch milliseconds — [`Session::heard`].
    heard: i64,
    /// Whether the transcript has been consulted for the session's origin —
    /// "there is no origin" against "we have not looked", which `asked: None` cannot
    /// say. A conversation continued from a compacted one correctly has none, and the
    /// next prompt must not be taken for it. Only a session with no transcript may
    /// be named by what it is told next.
    origin_read: bool,
    asked: Option<String>,
    stderr: String,
    /// Messages written to stdin that the CLI has not echoed back, oldest first. See
    /// [`Session::deaf`].
    unread: VecDeque<Unread>,
    /// Whether the session has spoken since its last turn ended — [`Summary::working`].
    /// A positive fact, not `idle_since.is_none()`, which is true for a session that
    /// has said nothing at all and reported a still-loading one as working.
    working: bool,
    /// When the last turn ended, in epoch milliseconds — `None` whenever the session
    /// is working. See [`Session::deaf`].
    idle_since: Option<i64>,
    /// Whether this episode of deafness has already been announced. See
    /// [`Session::check_deaf`].
    announced_deaf: bool,
    /// When the oldest decision the session has not acted on was written, in epoch
    /// milliseconds. See [`Session::deaf`].
    decided: Option<i64>,
    /// Slash commands written while a turn was running, oldest first. A command sent
    /// mid-turn does not run: the CLI parks it as a `queued_command` with
    /// `commandMode: "prompt"` and hands it to the MODEL as words. Held here rather
    /// than in the client, which stops holding it when the phone is put away. See
    /// [`Session::send`] and [`Session::release_held`].
    held: VecDeque<String>,
    /// A `/compact` has been sent and the conversation has not moved since — the one
    /// long silence that is not a fault. See [`Session::deaf`].
    compacting: bool,
}

/// How long a message may sit unread, between turns and with the session
/// otherwise silent, before the session is called deaf. The legitimate wait is
/// seconds; the failures stayed silent for tens of minutes. Ninety seconds sits
/// an order of magnitude clear of each.
const DEAF_AFTER_MS: i64 = 90_000;

/// The same wait while a compaction is outstanding, which freezes the transcript
/// for minutes. Longer rather than suppressed: a session can go deaf AROUND a
/// compaction, and one of the two known episodes did.
const DEAF_AFTER_COMPACT_MS: i64 = 15 * 60_000;

/// Drop one piece of background work from the count, by whichever name the thing
/// that ended it knew: a removal for a call, a search for a task.
fn forget(
    background: &mut std::collections::BTreeMap<String, crate::protocol::Called>,
    named: &crate::protocol::Named,
) {
    match named {
        crate::protocol::Named::Call(call) => {
            background.remove(call);
        }
        crate::protocol::Named::Task(task) => {
            background.retain(|_, started| started.task.as_deref() != Some(task.as_str()));
        }
    }
}

/// Keep track of what is in flight, and whether the session can read it.
/// Everything [`Session::deaf`] decides on is maintained here, in one place: the
/// verdict is a conjunction, and a field updated in only some arms fails silently.
fn in_flight(state: &mut State, event: &Event) {
    match event {
        // The read receipt. Oldest match first: stdin is a queue.
        Event::Prompt { text } => {
            if let Some(at) = state.unread.iter().position(|held| &held.text == text) {
                state.unread.remove(at);
            }
            // It read something, so this episode is over; a next one is worth saying again.
            state.announced_deaf = false;
        }
        // A turn ended, so from here the session owes us a read.
        Event::Turn { .. } => state.idle_since = Some(now()),
        // Nothing is running as of now. `Joined` matters most: pushed AFTER the seeded
        // transcript, it stops a file ending mid-turn from reading as a turn still
        // going in a process that has only just started.
        Event::Started { .. } | Event::Joined { .. } => state.idle_since = Some(now()),
        _ => {}
    }
    state.working = working_after(state.working, event);
    match event {
        Event::Command { text } if text.starts_with("/compact") => state.compacting = true,
        // A decision written down the pipe of a session that ASKED for it. Its own
        // clock: a session blocked on a question is mid-turn, so the message test
        // cannot fire. Silence after this is not work.
        Event::Answered { .. } => state.decided = state.decided.or(Some(now())),
        _ => {}
    }
    // Anything the session says of its own accord means it is working. Not `Busy`:
    // a status is announced only when it changes. A `Turn` counts but must not
    // clear `idle_since`, which it has just set.
    if matches!(
        event,
        Event::Text { .. }
            | Event::Tool { .. }
            | Event::ToolResult { .. }
            | Event::Context { .. }
            | Event::Prompt { .. }
            | Event::Turn { .. }
    ) {
        state.compacting = false;
        // Taken up: the tool it asked about has run, or the turn moved on.
        state.decided = None;
        if !matches!(event, Event::Turn { .. }) {
            state.idle_since = None;
        }
    }
}

/// Whether a turn is running, after `event`. Set by the session speaking,
/// cleared when the turn ends, and false until it has ever spoken. `Started` and
/// `Joined` clear it too, or a resumed session whose file ends mid-turn reads as
/// working. Public because reaching that case in a test needs only this.
pub fn working_after(was: bool, event: &Event) -> bool {
    match event {
        Event::Text { .. }
        | Event::Tool { .. }
        | Event::ToolResult { .. }
        | Event::Context { .. }
        | Event::Prompt { .. } => true,
        Event::Turn { .. }
        | Event::Exited { .. }
        | Event::Started { .. }
        | Event::Joined { .. } => false,
        // Everything else — a status, a decision, a command — leaves the answer where it was.
        _ => was,
    }
}

/// The verdict itself, against state a caller is already holding — see
/// [`Session::deaf`].
fn deaf_for(state: &State) -> Option<i64> {
    if !state.alive {
        return None;
    }
    deaf_after(
        state.idle_since,
        state.unread.front().map(|held| held.at),
        state.decided,
        state.compacting,
        now(),
    )
}

/// The verdict as arithmetic. Public because it is the part worth testing without
/// waiting ninety seconds.
///
/// * `idle_since` — when the last turn ended, `None` while the session works.
/// * `oldest` — when the oldest unread message was written.
/// * `decided` — when the oldest unacted-on decision was written; needs no
///   `idle_since`, since the session asked and stopped.
///
/// Two ways to be waiting, and either is enough; whichever has waited longer is
/// reported.
pub fn deaf_after(
    idle_since: Option<i64>,
    oldest: Option<i64>,
    decided: Option<i64>,
    compacting: bool,
    now: i64,
) -> Option<i64> {
    // The LATER of the two: only once the turn has ended AND the message has
    // arrived has the session had the chance this measures.
    let unread_since = idle_since.zip(oldest).map(|(idle, at)| idle.max(at));
    // The EARLIER of the two cases, so a long wait is not hidden by a short one.
    let since = [unread_since, decided].into_iter().flatten().min()?;
    let allowed = if compacting {
        DEAF_AFTER_COMPACT_MS
    } else {
        DEAF_AFTER_MS
    };
    let waited = now - since;
    (waited >= allowed).then_some(waited)
}

/// A message written to the session's stdin that it has not read back:
/// [`Event::Accepted`] says the bytes reached the pipe, the CLI's replay says
/// they were taken out. Commands are absent — the CLI never replays one.
#[derive(Debug, Clone)]
struct Unread {
    text: String,
    /// When it was written, in epoch milliseconds.
    at: i64,
}

pub struct Session {
    pub id: String,
    pub dir: PathBuf,
    started: SystemTime,
    state: Mutex<State>,
    stdin: tokio::sync::Mutex<Option<Sink>>,
    /// The process id, kept because an adopted session has no [`Child`] handle.
    pid: u32,
    /// See [`Fds`]. Kept for the same reason.
    fds: Fds,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    tx: broadcast::Sender<Stamped>,
}

/// By hand because the sink is a trait object, and the identity is the
/// interesting half anyway.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("dir", &self.dir)
            .field("pid", &self.pid)
            .field("fds", &self.fds)
            .finish_non_exhaustive()
    }
}

/// How to spawn the CLI. Held by the roster and handed to each session.
#[derive(Debug, Clone)]
pub struct Spawn {
    pub binary: String,
    pub model: Option<String>,
    /// What this session is called to its PEERS — `-n`, which is the only thing that
    /// writes a chosen name into `~/.claude/sessions/<pid>.json`, the file
    /// `ListAgents` reads and `SendMessage` addresses. Without it every session in
    /// one directory derives the same name from that directory and is told apart
    /// only by a hash, so a session asked to reach `memview` finds no such peer.
    /// `rename_session` does NOT do this: the CLI calls it the user-facing title and
    /// it goes no further than the transcript. See [`crate::roster::Roster::resume`],
    /// which is where a conversation's own name becomes this argument.
    pub name: Option<String>,
    /// What the session may do without being asked. In headless mode there is nobody
    /// to answer a prompt, so under the CLI's default EVERY tool call needing
    /// permission is refused.
    pub permission_mode: Option<String>,
}

impl Session {
    /// Start a session in `dir`, with `id` as both our handle and its session id.
    pub fn start(id: String, dir: &Path, spawn: &Spawn) -> Result<Arc<Self>> {
        Self::spawn(id, dir, spawn, false)
    }

    /// Pick up a conversation that already exists, keeping its id. `--resume` rather
    /// than `--session-id`: an id the CLI has never seen is an error, not a fresh
    /// session wearing a real one's name. The transcript is not a lock — a `claude`
    /// in a terminal is invisible to the roster.
    pub fn resume(id: String, dir: &Path, spawn: &Spawn) -> Result<Arc<Self>> {
        // Nothing recorded here about the file's date: resuming appends `mode`,
        // `permission-mode` and `bridge-session` lines, so a floor taken from the file
        // said `just now` about a conversation opened after two days. See
        // [`crate::past::last_moved`].
        let session = Self::spawn(id, dir, spawn, true)?;
        // A NEW `claude` on an old conversation: whatever the transcript shows still
        // running was written by a process that is gone.
        session.seed(true);
        Ok(session)
    }

    /// Put what was already said in front of what happens next. `--resume` restores
    /// the CLI's context and replays none of it on stdout, so without this a resumed
    /// session opened empty. The same vocabulary the stream uses, read the other way
    /// — [`crate::protocol::read_recorded`]. Silent when there is no transcript.
    fn seed(self: &Arc<Self>, restarted: bool) {
        let root = crate::past::projects_root();
        let Some(path) = crate::past::transcript_of(&root, &self.id) else {
            tracing::info!(
                "no transcript found for {} — resuming with an empty view",
                self.id
            );
            return;
        };
        let seed = crate::past::page(&path, None);
        tracing::info!(
            "seeded {} with {} events from its transcript, from byte {}",
            self.id,
            seed.events.len(),
            seed.from
        );
        let count = seed.events.len();
        for timed in seed.events {
            self.push_at(timed.event, timed.at);
        }
        // Last, so it sits between what was read and what we watch, carrying the cursor.
        // Stamped now, because joining is the one thing here that did happen now.
        self.push(Event::Joined {
            earlier: count,
            from: seed.from,
            restarted,
        });
        // From the head of the file, overriding whatever the page set: the replay is
        // the LAST page and full of prompts. Set unconditionally, including to `None`;
        // `origin_read` is what makes `None` stick. See [`crate::past::opening`].
        {
            let mut state = self.state.lock();
            state.asked = crate::past::opening(&path);
            state.origin_read = true;
        }
        self.recount();
    }

    /// Count the exchanges the transcript has gained — [`crate::past::counted`].
    /// Reads the file OUTSIDE the lock, which is why this is a method and not a line
    /// in `push_at`. Safe from the reading task only: it reads the offset, then the
    /// file, then writes both back.
    fn recount(&self) {
        let root = crate::past::projects_root();
        let Some(path) = crate::past::transcript_of(&root, &self.id) else {
            return;
        };
        let mut so_far = self.state.lock().counted;
        // A seed arrives at zero, and zero is the whole file — gigabytes, on the
        // executor. Start where the last megabyte begins; [`crate::past::seed_from`]
        // says why the two agree exactly.
        if so_far.through == 0 {
            so_far.through = crate::past::seed_from(&path);
        }
        let found = crate::past::counted(&path, so_far);
        let mut state = self.state.lock();
        state.counted = found.counted;
        // Work the harness reported finished, closing the count here because there is
        // no event — see [`crate::past::Appended::finished`].
        for named in &found.finished {
            forget(&mut state.background, named);
        }
        // A compaction is announced in the file and nowhere else, so this read is the
        // only way a running session learns its fullness is stale. Taken from the file
        // rather than cleared: the same bytes may carry a request made after the boundary.
        if found.compacted {
            state.context = found.context;
        }
    }

    fn spawn(id: String, dir: &Path, spawn: &Spawn, resuming: bool) -> Result<Arc<Self>> {
        let mut command = Command::new(&spawn.binary);
        command
            .current_dir(dir)
            .arg("-p")
            // stream-json output is refused without --verbose: the CLI's rule.
            .arg("--verbose")
            .args(["--input-format", "stream-json"])
            .args(["--output-format", "stream-json"])
            .arg("--include-partial-messages")
            // The echo of our own prompt is how a client knows the message landed.
            .arg("--replay-user-messages")
            .args(if resuming {
                ["--resume", &id]
            } else {
                ["--session-id", &id]
            })
            // The switch that makes approvals possible at all; `--help` does not list it,
            // the TypeScript SDK uses it. Without it a session in `manual` mode
            // refuses every tool call outright and no question ever reaches the client.
            .args(["--permission-prompt-tool", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Without this the child keeps running when the console is killed.
            .kill_on_drop(true);
        if let Some(model) = &spawn.model {
            command.args(["--model", model]);
        }
        if let Some(name) = &spawn.name {
            command.args(["-n", name]);
        }
        if let Some(mode) = &spawn.permission_mode {
            command.args(["--permission-mode", mode]);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning {} in {}", spawn.binary, dir.display()))?;
        // Read before the handles are moved out: after the upgrade these numbers are
        // all that is left of the connection.
        let fds = Fds {
            stdin: child.stdin.as_ref().map_or(-1, AsRawFd::as_raw_fd),
            stdout: child.stdout.as_ref().map_or(-1, AsRawFd::as_raw_fd),
            stderr: child.stderr.as_ref().map_or(-1, AsRawFd::as_raw_fd),
        };
        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take().context("child has no stdin")?;
        let (kill_tx, kill_rx) = oneshot::channel();
        let (tx, _) = broadcast::channel(256);

        let session = Arc::new(Self {
            id,
            dir: dir.to_path_buf(),
            started: SystemTime::now(),
            state: Mutex::new(State {
                alive: true,
                // What was actually asked for. Unset is not unknown — it is the CLI's own
                // default, under which every tool call needing permission comes back here.
                mode: Some(
                    spawn
                        .permission_mode
                        .clone()
                        .unwrap_or_else(|| DEFAULT_MODE.to_string()),
                ),
                ..State::default()
            }),
            stdin: tokio::sync::Mutex::new(Some(Box::new(stdin) as Sink)),
            pid,
            fds,
            kill: Mutex::new(Some(kill_tx)),
            tx,
        });

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        session.clone().read_from(stdout, stderr, false);
        session.clone().reap(child, kill_rx);
        Ok(session)
    }

    /// Take over a session the previous image was running: same pid, same pipes,
    /// same conversation. What is lost is the [`Child`] handle, so exit is noticed by
    /// end of file and killing goes through the pid. The scrollback does not survive;
    /// a reconnecting client is reseeded from the transcript, and [`Tally`] is carried.
    pub fn adopt(
        id: String,
        dir: PathBuf,
        pid: u32,
        fds: Fds,
        mut tally: Tally,
    ) -> Result<Arc<Self>> {
        let pending = std::mem::take(&mut tally.pending);
        // Taken out for the same reason as `pending`, and put back after the seed.
        let background = std::mem::take(&mut tally.background);
        // The scrollback did NOT survive the exec, so an adopted session reseeds from
        // the transcript exactly as a resumed one does — see [`Self::seed`].
        // SAFETY: these descriptors were handed over by the image that exec'd, which
        // cleared close-on-exec so they would survive.
        let (stdin, stdout, stderr) = unsafe {
            (
                OwnedFd::from_raw_fd(fds.stdin),
                OwnedFd::from_raw_fd(fds.stdout),
                OwnedFd::from_raw_fd(fds.stderr),
            )
        };
        let stdin = tokio::net::unix::pipe::Sender::from_owned_fd(stdin)
            .context("adopting the session's stdin")?;
        let stdout = tokio::net::unix::pipe::Receiver::from_owned_fd(stdout)
            .context("adopting the session's stdout")?;
        let stderr = tokio::net::unix::pipe::Receiver::from_owned_fd(stderr)
            .context("adopting the session's stderr")?;
        let (kill_tx, _kill_rx) = oneshot::channel();
        let (tx, _) = broadcast::channel(256);

        let session = Arc::new(Self {
            id,
            dir,
            // A tally from an image that never counted a turn has a zero here.
            started: match tally.started {
                0 => SystemTime::now(),
                secs => UNIX_EPOCH + Duration::from_secs(secs),
            },
            state: Mutex::new(State {
                alive: true,
                model: tally.model,
                mode: tally.mode,
                // Before the seed runs, so the replay cannot overwrite it: `asked` is only ever
                // set when `None`. See the field's note in [`Tally`].
                asked: tally.asked,
                cost_usd: tally.cost_usd,
                window: tally.window,
                limit: tally.limit,
                // Carried like the rest of the tally: nothing on disk records it.
                spent: tally.spent,
                counted: tally.counted,
                // The turn that was in flight is still in flight; its next status line or
                // `Turn` corrects this.
                busy: tally.busy,
                ..State::default()
            }),
            stdin: tokio::sync::Mutex::new(Some(Box::new(stdin) as Sink)),
            pid,
            fds,
            kill: Mutex::new(Some(kill_tx)),
            tx,
        });
        // The SAME child across the exec, so nothing above the boundary is dead on
        // account of the upgrade — see [`crate::protocol::Event::Joined`].
        session.seed(false);
        // After the seed, so the question lands at the end of the conversation, where
        // it is still standing. Pushed as an ordinary `Ask`: one mechanism, so a
        // reconnecting client is offered the decision again and the session is
        // recorded as waiting for it.
        for (id, question) in pending {
            session.push(Event::Ask {
                id,
                call: question.call,
                tool: question.tool,
                title: question.title,
                detail: question.detail,
                input: question.input,
            });
        }
        // After the seed, because the seed ends with a `Joined`, which
        // `protocol::running` reads as `Running::Gone` — right for a resume, wrong for
        // an adoption whose children are still running. `tests/cold.rs` caught it.
        if !background.is_empty() {
            tracing::info!(
                "{}: {} background task(s) carried across the upgrade",
                session.id,
                background.len()
            );
            session.state.lock().background = background;
        }
        session.clone().read_from(Some(stdout), Some(stderr), true);
        reap_adopted(pid);
        Ok(session)
    }

    /// What this session has counted, for an upgrade to hand on. See [`Tally`].
    pub fn tally(&self) -> Tally {
        let state = self.state.lock();
        Tally {
            started: self
                .started
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model: state.model.clone(),
            cost_usd: state.cost_usd,
            window: state.window,
            limit: state.limit.clone(),
            spent: state.spent.clone(),
            mode: state.mode.clone(),
            asked: state.asked.clone(),
            busy: state.busy.clone(),
            pending: state.pending.clone(),
            background: state.background.clone(),
            counted: state.counted,
        }
    }

    /// Questions this session is still waiting on. A cold reader has to be offered
    /// these again: an `Ask` is a control request no transcript records, so a seed
    /// cannot contain one — `crate::api::cold` needs this as [`Session::adopt`] does.
    pub fn asking(&self) -> Vec<(String, Pending)> {
        self.state
            .lock()
            .pending
            .iter()
            .map(|(id, question)| (id.clone(), question.clone()))
            .collect()
    }

    /// The pipes to this session's process, for an upgrade to hand on.
    pub fn fds(&self) -> Fds {
        self.fds
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Read the child's streams until they end. `ends_on_eof` decides who declares
    /// the session over: for an adopted session end of file is the only signal; for
    /// a spawned one [`Self::reap`] must say so, because only it knows the exit code,
    /// and letting the reader win turned every clean exit into `code: None`.
    fn read_from<O, E>(self: Arc<Self>, stdout: Option<O>, stderr: Option<E>, ends_on_eof: bool)
    where
        O: tokio::io::AsyncRead + Unpin + Send + 'static,
        E: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        if let Some(stdout) = stdout {
            let session = self.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                let mut beat = tokio::time::interval(RECOUNT_EVERY);
                // Delay, not Burst: a session busy for a minute owes one catch-up read.
                beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    let line = tokio::select! {
                        line = lines.next_line() => line,
                        // The transcript changes when the process says nothing: a compaction is written
                        // to the file and announced on no stream, so a read tied to `Turn` left `home`
                        // showing a stale fullness for ninety minutes.
                        _ = beat.tick() => {
                            session.recount();
                            continue;
                        }
                    };
                    let Ok(Some(line)) = line else { break };
                    // Every line, whatever it is: the record of when the PROCESS last spoke, which
                    // is how the roster decides who to ask for usage. See [`Session::heard`].
                    session.heard();
                    // Before the events and separately: a control response is an answer to
                    // something the console asked, not something that happened in the conversation.
                    if let Some(windows) = protocol::usage_reply(&line) {
                        session.record_usage(windows);
                        continue;
                    }
                    // The other answer this console asks for — see [`Session::settle_mode`].
                    if let Some(reply) = protocol::mode_reply(&line) {
                        session.settle_mode(reply);
                        continue;
                    }
                    for event in protocol::read(&line) {
                        // The end of a turn is the one moment the exchange count can have changed, and
                        // the CLI has written the whole exchange by then. Here rather than in `push_at`,
                        // which holds the state lock and must not read files; the count is wanted NOW.
                        let counted = matches!(event, Event::Turn { .. });
                        session.push(event);
                        if counted {
                            session.recount();
                            // The moment the commands parked mid-turn have been waiting for. Here for the
                            // same reason as the recount: this writes to a pipe. A failure is logged, not
                            // propagated — ending this loop would take the transcript with it.
                            if let Err(err) = session.release_held().await {
                                tracing::warn!("{}: holding a command back: {err:#}", session.id);
                            }
                        }
                    }
                }
                // The pipe closed, so the process did — but only say so when nothing better is
                // watching.
                if ends_on_eof {
                    session.ended(None);
                }
            });
        }
        if let Some(stderr) = stderr {
            let session = self.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    session.note_stderr(&line);
                }
            });
        }
    }

    /// Wait for a spawned child, and kill it when asked. The adopted half is
    /// [`reap_adopted`].
    fn reap(self: Arc<Self>, mut child: Child, kill: oneshot::Receiver<()>) {
        tokio::spawn(async move {
            let code = tokio::select! {
                status = child.wait() => status.ok().and_then(|s| s.code()),
                _ = kill => {
                    let _ = child.kill().await;
                    None
                }
            };
            self.ended(code);
        });
    }

    /// Record that the process is gone, once — the reader and the reaper can both
    /// notice.
    fn ended(&self, code: Option<i32>) {
        {
            let mut state = self.state.lock();
            if !state.alive {
                return;
            }
            state.alive = false;
        }
        self.push(Event::Exited { code });
    }

    /// Send a message to the session. A slash command sent mid-turn is HELD, not
    /// written — see [`State::held`] — under the same lock [`in_flight`] takes to
    /// clear `working`, so a turn ending beside this either drains what was parked
    /// or lets this write straight through.
    pub async fn send(&self, text: &str) -> Result<()> {
        let parked = {
            let mut state = self.state.lock();
            let parking = state.working && protocol::is_command(text);
            if parking {
                state.held.push_back(text.to_string());
            }
            parking
        };
        if parked {
            return Ok(());
        }
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{}\n", protocol::prompt(text)).as_bytes())
            .await
            .context("writing to the session")?;
        stdin.flush().await.context("flushing to the session")?;
        drop(held);
        // Announced on the way in, not on the echo: the CLI may not read it for
        // minutes — see [`Event::Accepted`]. Prompt or command is decided here, since
        // only the text can say which will be echoed. See [`Event::Command`].
        if protocol::is_command(text) {
            self.push(Event::Command {
                text: text.to_string(),
            });
        } else {
            // In flight until the CLI replays it — the whole of what [`Self::deaf`] has to go on.
            self.state.lock().unread.push_back(Unread {
                text: text.to_string(),
                at: now(),
            });
            self.push(Event::Accepted {
                text: text.to_string(),
            });
        }
        // Held even if the CLI never echoes it.
        let mut state = self.state.lock();
        if state.asked.is_none() && !state.origin_read {
            state.asked = Some(text.to_string());
        }
        Ok(())
    }

    /// Write the commands that were waiting for this turn to end, through
    /// [`Self::send`], so each takes the ordinary path and is recorded as it goes.
    /// One at a time, re-locked between each, so a cancel mid-drain is honoured. A
    /// failed write stops the drain: writing the next after a refusal would be pretending.
    pub async fn release_held(&self) -> Result<()> {
        loop {
            let next = {
                let mut state = self.state.lock();
                state.held.pop_front()
            };
            let Some(command) = next else { return Ok(()) };
            self.send(&command).await?;
        }
    }

    /// Take back a command that is waiting, by its exact text. False is not an
    /// error: a second tap on a command already released has nothing to undo.
    pub fn forget_held(&self, text: &str) -> bool {
        let mut state = self.state.lock();
        let Some(at) = state.held.iter().position(|held| held == text) else {
            return false;
        };
        state.held.remove(at);
        true
    }

    /// Show the session a picture, with whatever was said about it. Not folded into
    /// [`Self::send`]: the two differ on the wire ([`protocol::prompt_with_image`]).
    pub async fn show(
        &self,
        text: &str,
        media_type: &str,
        base64: &str,
        kept: &std::path::Path,
    ) -> Result<()> {
        let line = protocol::prompt_with_image(text, media_type, base64, kept);
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("writing to the session")?;
        stdin.flush().await.context("flushing to the session")?;
        drop(held);
        let mut state = self.state.lock();
        if state.asked.is_none() && !state.origin_read {
            // What it was opened for, when a picture is the first thing said — not the
            // base64.
            state.asked = Some(match text.trim() {
                "" => "an image".to_string(),
                words => words.to_string(),
            });
        }
        Ok(())
    }

    /// Answer a question the session is blocked on. Refusing carries a reason the
    /// session can act on. An unknown id is an error: the likeliest cause is two
    /// people looking at one session. `reply` is what was said about a
    /// [`protocol::QUESTION_TOOL`] call, refused for anything else — a console whose
    /// job is approving tool calls should not also rewrite what it approved.
    pub async fn decide(
        &self,
        id: &str,
        allowed: bool,
        why: &str,
        reply: Option<&protocol::Reply>,
    ) -> Result<()> {
        let pending = {
            let state = self.state.lock();
            state
                .pending
                .get(id)
                .cloned()
                .context("that question is not open — it may already have been answered")?
        };
        if reply.is_some() && pending.tool != protocol::QUESTION_TOOL {
            anyhow::bail!(
                "answers were sent for {}, which does not ask questions",
                pending.tool
            );
        }
        let line = protocol::decision(id, allowed, &pending.input, why, reply);
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("answering the session")?;
        stdin.flush().await.context("flushing the answer")?;
        drop(held);
        self.push(Event::Answered {
            id: id.to_string(),
            allowed,
            reply: reply.cloned(),
        });
        Ok(())
    }

    /// Rename the conversation — a control request, not `/rename`, which a working
    /// session hands to the MODEL as words. See [`protocol::rename`]. Nothing is
    /// recorded on the way out: the CLI writes a `custom-title` line and the roster
    /// reads names from there ([`crate::past::about`]).
    ///
    /// This is the TITLE, and it does not reach the session's peers. The CLI
    /// keeps the name other sessions address separately, and no control subtype
    /// writes it — only `-n` at spawn does, so a rename becomes visible to the rest
    /// of the fleet at the conversation's next resume and not before. See
    /// [`Spawn::name`].
    pub async fn rename(&self, title: &str) -> Result<()> {
        let line = protocol::rename(&format!("rename-{}", self.id), title);
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("renaming the session")?;
        stdin.flush().await.context("flushing the rename")?;
        Ok(())
    }

    /// Change what this session may do without asking. Recorded optimistically —
    /// the CLI's answer is not waited for, see [`protocol::set_mode`] — and only
    /// after stdin has taken the line, so a failed write leaves the true mode on screen.
    pub async fn set_mode(&self, mode: &str) -> Result<()> {
        let line = protocol::set_mode(&format!("set-mode-{}", self.id), mode);
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("asking the session to change mode")?;
        stdin.flush().await.context("flushing the mode change")?;
        drop(held);
        let mut state = self.state.lock();
        // Kept so a refusal can put back the mode the session is really in. Only when
        // nothing is already outstanding, or the second change would record the first's
        // optimistic value as the truth.
        if state.restore.is_none() {
            state.restore = state.mode.clone();
        }
        // The old explanation goes with the old attempt.
        state.mode_refused = None;
        state.mode = Some(mode.to_string());
        Ok(())
    }

    /// Take the CLI at its word about what mode it is in — the correction
    /// [`Session::set_mode`]'s optimism depends on, or a refused mode stays on
    /// screen. The confirmed mode comes
    /// from the reply, not from what was asked — see [`protocol::mode_reply`].
    fn settle_mode(&self, reply: protocol::ModeReply) {
        let mut state = self.state.lock();
        match reply {
            protocol::ModeReply::Now(mode) => {
                state.mode = Some(mode);
                state.mode_refused = None;
            }
            protocol::ModeReply::Refused(why) => {
                tracing::info!("{}: the mode change was refused — {why}", self.id);
                // Back to what it was. `restore` is empty only for a reply to a change this
                // console did not make.
                if let Some(was) = state.restore.clone() {
                    state.mode = Some(was);
                }
                state.mode_refused = Some(why);
            }
        }
        state.restore = None;
    }

    /// Keep what the CLI answered about each window. Per window, not wholesale: an
    /// answer naming one window says nothing about another.
    fn record_usage(&self, windows: Vec<(String, f64, Option<i64>)>) {
        let mut state = self.state.lock();
        let at = Heard(now());
        // Through [`crate::usage::remember`], not a blind insert: an answer from cached
        // headers is an echo, and must not overwrite a fresh `rate_limit_event`.
        crate::usage::remember(
            &mut state.spent,
            windows.into_iter().map(|(window, utilization, resets_at)| {
                (
                    window,
                    Seen {
                        utilization,
                        resets_at: resets_at.map(ResetsAt),
                        at,
                        measured: false,
                    },
                )
            }),
        );
    }

    /// Ask this session what the account has spent — [`protocol::get_usage`]. The
    /// answer arrives on stdout and is recorded as it passes [`Self::record_usage`].
    /// Failure is not propagated: a session that will not take the question is one
    /// whose figures the console does not have.
    pub async fn ask_usage(&self) {
        let line = protocol::get_usage(&format!("usage-{}", self.id));
        let mut held = self.stdin.lock().await;
        let Some(stdin) = held.as_mut() else { return };
        if stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .is_ok()
        {
            let _ = stdin.flush().await;
        }
    }

    /// End the session: close stdin, and kill it if it has not gone on its own.
    /// The deadline is recorded as well as slept on, because the sleep does not
    /// survive an upgrade: `handover` re-execs this process and the sleep is lost.
    /// [`crate::roster::Roster::finish_stopping`] reads it.
    pub async fn stop(self: &Arc<Self>) {
        self.stdin.lock().await.take();
        self.state.lock().stopping = Some(now() + GRACE.as_millis() as i64);
        let session = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(GRACE).await;
            session.force();
        });
    }

    /// When this session's kill falls due, for one that has been stopped.
    pub fn stopping(&self) -> Option<i64> {
        self.state.lock().stopping
    }

    /// Kill the session now.
    pub fn force(&self) {
        if let Some(kill) = self.kill.lock().take() {
            // A spawned session: the reaper holds the child and kills it.
            if kill.send(()).is_ok() {
                return;
            }
        }
        // An adopted one has no child handle, so the pid is the only handle left.
        if self.pid != 0 {
            // SAFETY: a kill to a pid this console started; ESRCH is ignored.
            unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL) };
        }
    }

    /// What a call will change in the files it writes. See [`crate::edits`].
    pub fn edited(&self, edited: crate::edits::Edited) {
        self.push(Event::Edited {
            call: edited.call,
            hunks: edited.hunks,
        });
    }

    /// A call whose files did not end up as predicted. See [`crate::edits`].
    pub fn diverged(&self, diverged: crate::edits::Diverged) {
        self.push(Event::Diverged {
            call: diverged.call,
            paths: diverged.paths,
        });
    }

    /// Record an event as having happened now.
    fn push(&self, event: Event) {
        self.push_at(event, Some(now()));
    }

    /// Note that the process said something, whatever it was. About the PROCESS,
    /// not the conversation: which one holds a current answer to `get_usage`. See
    /// [`crate::roster::Roster::ask_usage`].
    fn heard(&self) {
        self.state.lock().heard = now();
    }

    /// When this session's process last said anything. See [`Self::heard`].
    pub fn last_heard(&self) -> i64 {
        self.state.lock().heard
    }

    /// How long this session has been failing to read what was written to it, in
    /// milliseconds — `None` for one that is merely busy, or quiet. See [`crate::deaf`].
    ///
    /// Three things at once: a message is in flight; the session is between turns
    /// ([`State::idle_since`]) — one mid tool call parks input on purpose; and long
    /// enough, [`DEAF_AFTER_MS`] or [`DEAF_AFTER_COMPACT_MS`]. The clock starts at
    /// whichever came second. It cannot see a session that goes deaf mid-turn.
    pub fn deaf(&self) -> Option<i64> {
        deaf_for(&self.state.lock())
    }

    /// Say so, once, if this session has stopped reading: how long, when this is the
    /// call that noticed, and `None` on later sweeps of the same episode. The pid
    /// comes back with it so the caller can capture the process before the cure
    /// destroys it — [`crate::roster::Roster::watch_for_deafness`].
    pub fn check_deaf(&self) -> Option<(u64, usize)> {
        let mut state = self.state.lock();
        let seconds = (deaf_for(&state)? / 1000) as u64;
        if state.announced_deaf {
            return None;
        }
        state.announced_deaf = true;
        let unread = state.unread.len();
        drop(state);
        self.push(Event::Deaf { unread, seconds });
        Some((seconds, unread))
    }

    /// What this session was last told it may do without asking. See [`Summary::mode`].
    pub fn mode(&self) -> Option<String> {
        self.state.lock().mode.clone()
    }

    /// What was written to this session and never read, oldest first — the other
    /// half of the cure, since a restart loses the old pipe. See
    /// [`crate::roster::Roster::revive`].
    pub fn unread(&self) -> Vec<String> {
        self.state
            .lock()
            .unread
            .iter()
            .map(|held| held.text.clone())
            .collect()
    }

    /// Record an event and hand it to whoever is listening. `at` is passed rather
    /// than taken because a seeded event happened whenever the transcript says.
    fn push_at(&self, event: Event, at: Option<i64>) {
        let stamped = {
            let mut state = self.state.lock();
            match &event {
                Event::Started { model, .. } => state.model = Some(model.clone()),
                Event::Busy { status } => state.busy = Some(status.clone()),
                // The window, which only the result line declares.
                Event::Context { tokens } => state.context = Some(*tokens),
                // Everything the last measurement counted was replaced by a summary, so this
                // is another conversation's number. Cleared rather than estimated; the next
                // message brings a real one. Seen only in a replay — live, [`Self::recount`]
                // reads the file.
                Event::Compacted => state.context = None,
                Event::Turn {
                    cost_usd, window, ..
                } => {
                    if window.is_some() {
                        state.window = *window;
                    }
                    state.busy = None;
                    // Assigned, not added: `total_cost_usd` is the running total, and `+=` reached
                    // $59.32 against a true $12.35.
                    state.cost_usd = *cost_usd;
                    // `num_turns` is deliberately not read: it counts assistant messages, not
                    // exchanges. [`Self::recount`] answers that from the file.
                }
                Event::Limit {
                    window,
                    status,
                    resets_at,
                    utilization,
                } => {
                    state.limit = Some(status.clone());
                    // One window per event, so they are collected rather than replaced wholesale.
                    if let Some(spent) = utilization {
                        // A measurement: the API's own headers off a request that just completed, dated
                        // by the event's stamp on replay and `now()` live. An event with no stamp
                        // cannot be dated, which is the definition of an echo.
                        crate::usage::remember(
                            &mut state.spent,
                            [(
                                window.clone(),
                                Seen {
                                    utilization: *spent,
                                    resets_at: resets_at.map(ResetsAt),
                                    at: Heard(at.unwrap_or_else(now)),
                                    measured: at.is_some(),
                                },
                            )],
                        );
                    }
                }
                Event::Prompt { text } if state.asked.is_none() && !state.origin_read => {
                    state.asked = Some(text.clone());
                }
                Event::Ask {
                    id,
                    call,
                    tool,
                    input,
                    title,
                    detail,
                } => {
                    state.pending.insert(
                        id.clone(),
                        Pending {
                            tool: tool.clone(),
                            call: call.clone(),
                            input: input.clone(),
                            title: title.clone(),
                            detail: detail.clone(),
                        },
                    );
                }
                Event::Answered { id, .. } => {
                    state.pending.remove(id);
                }
                Event::Exited { .. } => {
                    state.busy = None;
                    // Nothing can be approved for a process that has gone, and a question left
                    // standing would keep saying the session is waiting.
                    state.pending.clear();
                    // Nor can a command be written to it: a chip promising one is about to run is
                    // the lie this mechanism exists to stop.
                    state.held.clear();
                }
                _ => {}
            }
            // Remember what each call IS, so a detached one can be named — `State::called`.
            if let Event::Tool { id, name, input } = &event {
                if state.called.len() >= CALLED_RING {
                    state.called.pop_front();
                }
                state
                    .called
                    .push_back((id.clone(), crate::protocol::called(name, input)));
            }
            // Work left running is decided where the events are read — [`protocol::running`].
            match protocol::running(&event) {
                protocol::Running::Began { tool, task } => {
                    // Unnamed rather than absent when the ring has rolled past it: that it is
                    // running is the fact worth keeping.
                    let mut named = state
                        .called
                        .iter()
                        .find(|(id, _)| *id == tool)
                        .map(|(_, called)| called.clone())
                        .unwrap_or_else(|| crate::protocol::Called {
                            tool: String::from("tool"),
                            label: None,
                            task: None,
                        });
                    named.task = task;
                    state.background.insert(tool, named);
                }
                protocol::Running::Ended(named) => forget(&mut state.background, &named),
                // By the task, because that is the only name a kill gives.
                protocol::Running::Killed(task) => {
                    forget(&mut state.background, &protocol::Named::Task(task));
                }
                protocol::Running::Gone => state.background.clear(),
                protocol::Running::Quiet => {}
            }
            in_flight(&mut state, &event);
            state.issued += 1;
            let stamped = Stamped {
                seq: state.issued,
                at,
                event,
            };
            if state.log.len() >= SCROLLBACK {
                state.log.pop_front();
            }
            state.log.push_back(stamped.clone());
            stamped
        };
        // An error here means nobody is listening, which is normal.
        let _ = self.tx.send(stamped);
    }

    fn note_stderr(&self, line: &str) {
        let mut state = self.state.lock();
        state.stderr.push_str(line);
        state.stderr.push('\n');
        if state.stderr.len() > STDERR_KEPT {
            let cut = state.stderr.len() - STDERR_KEPT;
            state.stderr = state.stderr.split_off(cut);
        }
    }

    /// The transcript so far, unnumbered — for asking what a session has done.
    pub fn history(&self) -> Vec<Event> {
        self.state
            .lock()
            .log
            .iter()
            .map(|stamped| stamped.event.clone())
            .collect()
    }

    /// What a client that says it holds everything through `after` still needs.
    /// Resuming is refused rather than approximated — see [resumable].
    pub fn since(&self, after: Option<u64>) -> Backlog {
        let state = self.state.lock();
        // With an empty log nothing is held, so the earliest number still honourable
        // is the next one to be issued.
        let held_from = state
            .log
            .front()
            .map_or(state.issued + 1, |first| first.seq);
        if let Some(after) = after
            && resumable(after, held_from, state.issued)
        {
            let events: Vec<Stamped> = state
                .log
                .iter()
                .filter(|stamped| stamped.seq > after)
                .cloned()
                .collect();
            let through = events.last().map_or(after, |last| last.seq);
            return Backlog {
                resumed: true,
                events,
                through,
            };
        }
        let events: Vec<Stamped> = state.log.iter().cloned().collect();
        let through = events.last().map_or(0, |last| last.seq);
        Backlog {
            resumed: false,
            events,
            through,
        }
    }

    /// The last sequence number this session has issued, for a client seeded from
    /// the transcript — see [`crate::api`]. Every event at or below it has been
    /// written, so nothing after it is sent twice.
    pub fn issued(&self) -> u64 {
        self.state.lock().issued
    }

    pub fn listen(&self) -> broadcast::Receiver<Stamped> {
        self.tx.subscribe()
    }

    /// What the child said on stderr, for when it will not start.
    pub fn trouble(&self) -> String {
        self.state.lock().stderr.clone()
    }

    pub fn alive(&self) -> bool {
        self.state.lock().alive
    }

    /// Whether a turn is running right now — [`Summary::working`], and NOT
    /// [`Summary::busy`], which cannot answer this.
    pub fn working(&self) -> bool {
        self.state.lock().working
    }

    pub fn summary(&self) -> Summary {
        let state = self.state.lock();
        Summary {
            id: self.id.clone(),
            dir: self.dir.display().to_string(),
            started: self
                .started
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            // Left for the roster, which reads the transcript once per listing.
            touched: None,
            bytes: None,
            alive: state.alive,
            model: state.model.clone(),
            busy: state.busy.clone(),
            working: state.working,
            interactions: state.counted.interactions,
            mode: state.mode.clone(),
            mode_refused: state.mode_refused.clone(),
            cost_usd: state.cost_usd,
            limit: state.limit.clone(),
            context: state.context,
            window: state.window,
            background: state.background.len(),
            running: state.background.values().cloned().collect(),
            asked: state.asked.clone(),
            // Filled in by the roster, which knows where the transcripts are.
            name: None,
            peer_name: None,
            waiting: state.pending.len(),
            unread: state.unread.len(),
            deaf: deaf_for(&state).map(|ms| (ms / 1000) as u64),
            held: state.held.iter().cloned().collect(),
        }
    }
}

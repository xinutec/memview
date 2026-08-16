//! One live Claude Code session: the subprocess, its transcript, its listeners.
//!
//! **A session is one long-lived process, not a chain of resumed ones.** Probed
//! against CLI 2.1.220: with `--input-format stream-json` the process serves
//! turn after turn on an open stdin, keeps one session id throughout, and exits
//! 0 when stdin closes. So the console holds the process open and writes to it,
//! which is what makes "send them a new instruction" a message rather than a
//! cold start — and it is why closing stdin is the polite way to end a session.
//!
//! **The id is ours, chosen before the process exists.** `--session-id` takes a
//! UUID we generate, so a session has a name the moment it is asked for rather
//! than once the CLI has announced itself — which means a client can subscribe
//! to a session that is still starting, and `--resume` later takes the same id.

use std::collections::{BTreeMap, VecDeque};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, oneshot};

use crate::protocol::{self, Event};

/// Where a session's instructions go.
///
/// A trait object because a session is built two ways: spawned, where this is
/// the child's own [`ChildStdin`], and **adopted across an upgrade**, where it
/// is the same pipe reopened from a raw file descriptor the previous image left
/// behind. Nothing downstream can tell the difference, which is the point.
type Sink = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// The three pipes to a session's process, by number.
///
/// Kept so they can outlive this image. `execve` preserves open descriptors and
/// the process id, so the children of an upgraded console are still its
/// children and still reachable through exactly these numbers — see
/// [`crate::roster::Roster::handover`].
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Fds {
    pub stdin: std::os::fd::RawFd,
    pub stdout: std::os::fd::RawFd,
    pub stderr: std::os::fd::RawFd,
}

/// What a session has counted, for an upgrade to hand on.
///
/// ⚠ **None of this is in the transcript.** The result line is where the cost,
/// the rate-limit status and the context window arrive, and it is a stream
/// artefact — no transcript in the corpus contains a single `"type":"result"`
/// line, so [`Session::seed`] cannot get any of it back. The model is the same
/// story: it is announced on the init line and nowhere in the file. So an adopted
/// session either carries these across or starts at zero and reads as a fresh
/// conversation that has done nothing.
///
/// How full the context is is deliberately absent, because the file holds it
/// better: it is on every assistant message, so a re-seed recovers it. Anything
/// derivable from the transcript is derived, because that survives a cold start
/// too — and this only survives an upgrade.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Tally {
    /// Seconds since the epoch. Carried because `execve` leaves the child's
    /// clock alone, so a session that has run for an hour must not claim to have
    /// started when its console was last upgraded.
    pub started: u64,
    pub model: Option<String>,
    pub cost_usd: f64,
    pub window: Option<u64>,
    pub limit: Option<String>,
    /// See [`Summary::mode`]. Carried because the console is the only thing
    /// that knows it — no file records it in a way that can be trusted.
    pub mode: Option<String>,
    /// What the session is doing, if anything. See [`Summary::busy`].
    ///
    /// ⚠ **A re-seed cannot recover this**, for the same reason it cannot
    /// recover [`Self::pending`]: the CLI announces a status on stdout when it
    /// *changes*, and nothing of the sort is written to the transcript. A
    /// session that was mid-turn when the console replaced itself came back
    /// reading `idle` on the front page and stayed that way until it next
    /// printed something — which, for one that had gone quiet to compact, was
    /// several minutes of saying nothing was happening while something was.
    #[serde(default)]
    pub busy: Option<String>,
    /// ⚠ **Questions the session is blocked on, which nothing else can recover.**
    /// A `can_use_tool` request is a control message, not a transcript line, so
    /// a re-seed cannot produce it — and the *session* stays blocked on it
    /// across an upgrade, because `execve` does not touch the child. Dropping
    /// these orphaned the question: the process waited for an answer whose
    /// request id no longer existed anywhere, the card vanished off every
    /// screen, and the row sat on "running" for ever. Measured on a live
    /// session that lost an hour that way.
    #[serde(default)]
    pub pending: BTreeMap<String, Pending>,
    /// What the API last said about each rate-limit window, keyed by the CLI's
    /// own name for it (`five_hour`, `seven_day`, …).
    ///
    /// ⚠ **Account-wide, so any session's reading is the truth for all of
    /// them** — it comes off the response headers of whichever request happened
    /// most recently, not from anything this session did. Kept per session only
    /// because that is where the stream arrives; the roster takes the newest.
    /// Carried across an upgrade for the same reason the rest of the tally is:
    /// nothing on disk records it.
    #[serde(default)]
    pub spent: BTreeMap<String, Seen>,
    /// The exchange count and how far into the transcript it accounts for.
    ///
    /// ⚠ **Carried for the cost, not because the file cannot say it.** The file
    /// can — by being read from the beginning, which for the largest transcript
    /// here is 2.1 GB and twenty-four seconds. An upgrade re-seeds every session
    /// at once, so dropping this would mean reading every transcript on the
    /// machine, four gigabytes of it, each time this console replaces itself.
    /// See [`crate::past::counted`].
    #[serde(default)]
    pub counted: crate::past::Counted,
}

/// When a rate-limit window turns over, in epoch **seconds** — the CLI's unit,
/// and nothing else in this console's.
///
/// The reading's own subject: it names *which instance* of the window a figure
/// belongs to, which is what makes two figures comparable at all.
///
/// A type rather than an `i64` because it sits beside [`Heard`] in [`Seen`] and
/// the two are neither the same clock nor the same unit. See
/// [`crate::usage::fresher`] for what went wrong when they were interchangeable.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ResetsAt(pub i64);

impl ResetsAt {
    /// The same instant in milliseconds, which is the unit of every other
    /// moment here. The one place the conversion is written.
    pub fn in_ms(self) -> i64 {
        self.0 * 1000
    }
}

/// When this console heard a reading, in epoch **milliseconds**, by this
/// machine's clock.
///
/// ⚠ **Arrival, not freshness.** Every session answers from its own process's
/// cached rate-limit headers, so a reading that arrives now can describe the
/// account as it stood an hour ago. Ordering by this is what
/// [`crate::usage::fresher`] exists to stop.
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
}

/// Wait on an adopted child, so the kernel can let go of it when it ends.
///
/// ⚠ **An adopted session has no [`Child`] to wait on.** The handle belonged to
/// the image that `execve`d away; only the pid and three descriptors crossed. So
/// nothing ever called `wait` on it, and every adopted session that ended left a
/// `<defunct>` entry behind a parent that would never ask. Measured 2026-08-12:
/// **22 of them** under a console up three days, against 16 the day before — one
/// per session that has ended since the first upgrade. They cost a process-table
/// slot each and nothing else; the reason to fix it is that the number only ever
/// goes up, because this process is deliberately never restarted (memview #753).
///
/// A **blocking** `waitpid` rather than a poll on a timer: the pid is this
/// process's own child, so the kernel already knows when to wake us and there is
/// nothing to choose an interval for. The thread it holds is released the moment
/// the child exits, and there is one per adopted session — a handful, against a
/// blocking pool of hundreds.
///
/// ⚠ **NOT `SIGCHLD` set to `SIG_IGN`**, which is the obvious cure and the wrong
/// one: it reaps every child automatically and takes the exit status with it.
/// [`Session::reap`] reads that status and [`Session::ended`] reports it, so a
/// spawned session's clean exit would start arriving as `code: None`, which
/// reads as *killed*. This file has made that mistake once and has a test
/// against it.
///
/// The status is deliberately dropped here rather than reported. For an adopted
/// session end-of-file is what declares the session over
/// ([`Session::read_from`]), and it has already fired by the time this returns;
/// making this the authority instead would be the same race that test guards.
fn reap_adopted(pid: u32) {
    tokio::task::spawn_blocking(move || {
        let mut status = 0;
        // SAFETY: `pid` is a child of this process — it was one of the previous
        // image's, and `execve` does not change parentage. `waitpid` only reads
        // that child's exit status.
        if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) } < 0 {
            // ECHILD is the ordinary case for a session already gone and reaped
            // by an earlier image of this console; nothing is wrong and there is
            // nothing to do.
            tracing::debug!(
                "adopted {pid} could not be waited on: {}",
                std::io::Error::last_os_error()
            );
        }
    });
}

/// Take a descriptor out of close-on-exec, and make it non-blocking.
///
/// ⚠ **Rust sets `O_CLOEXEC` on every pipe it creates**, so without the first
/// half an upgraded image inherits nothing and every live session is silently
/// unreachable. The second half is tokio's requirement for adopting a pipe.
///
/// Returns false when the descriptor is not there any more, which is the honest
/// answer for a session whose process has already gone.
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

/// Whether a `ps` listing shows this conversation still being run.
///
/// The words are read through [`crate::past::words_of_claude_processes`], so a
/// line that merely *mentions* the id — a grep, an editor, this console — is not
/// a `claude`, for the reason written down there.
pub fn names_session(ps_output: &str, id: &str) -> bool {
    crate::past::words_of_claude_processes(ps_output)
        .iter()
        .any(|word| word == id)
}

/// Kill a stopped session's process, after checking it is still that process.
///
/// ⚠ **A pid is not a handle, and this one is up to thirty seconds old.** The
/// warning is already written down at [`crate::roster::Roster::revive`]: a late
/// SIGKILL aimed at a pid the console no longer owns lands on whatever the
/// system started in its place, and that is the kind of fault nothing can trace
/// afterwards. So the pid is confirmed to still be running this conversation
/// before anything is sent to it.
///
/// Anything unreadable — no `ps`, no permission, no such process — is *not* a
/// kill. Leaving a process alive is the recoverable half of this decision; the
/// other half is unbounded.
pub fn finish(pid: u32, id: &str) {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
    else {
        tracing::warn!("could not ask ps about {pid}, so {id} was left alone");
        return;
    };
    if !names_session(&String::from_utf8_lossy(&output.stdout), id) {
        // The ordinary case, and the one worth logging at info: the session took
        // its stdin closing as the exit it is and went on its own.
        tracing::info!("{id} had already gone, so pid {pid} was left alone");
        return;
    }
    tracing::info!("{id} outlived its grace period — killing pid {pid}");
    // SAFETY: a kill to a pid this console started and has just confirmed is
    // still running that session; ESRCH for one that went in between is ignored.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// How much transcript one session keeps in memory.
///
/// The console holds no database in phase 1 and the transcripts on disk are the
/// durable record, so this is a scrollback, not an archive.
const SCROLLBACK: usize = 5000;

/// How many recent tool calls are remembered so a detached one can be named.
///
/// ⚠ **Small on purpose.** The `Tool` event naming a call is immediately
/// followed by the `ToolResult` that reveals it detached, so anything beyond a
/// handful is never consulted — and this is per session, held for the life of a
/// process that runs for days. 32 covers a turn that fans out several calls
/// before any of them answers.
const CALLED_RING: usize = 32;

/// How long a session gets to finish after its stdin closes, before it is killed.
///
/// Generous on purpose: the process may be mid-tool-call, and the clean exit is
/// worth waiting for because it is the one that flushes the transcript.
const GRACE: Duration = Duration::from_secs(30);

/// How much of the child's stderr to keep for diagnosis.
const STDERR_KEPT: usize = 4000;

/// How often to re-read the transcript while the child says nothing.
///
/// The read is incremental — a seek to where the last one stopped, then whatever
/// has been appended, which for an idle session is nothing at all. So this is a
/// handful of syscalls per session, and the interval is set by how long a wrong
/// number may stay on screen rather than by what the read costs: about as long
/// as it takes to look at the card and read it.
const RECOUNT_EVERY: Duration = Duration::from_secs(5);

/// The CLI's own name for the mode a session runs in when nothing is passed.
///
/// Displayed as *Manual*: it asks before every tool call that needs permission,
/// which in headless mode means asking whoever is holding the phone.
pub const DEFAULT_MODE: &str = "default";

/// What a client sees of a session without reading its transcript.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub id: String,
    pub dir: String,
    /// Seconds since the epoch.
    pub started: u64,
    /// When anything last happened, in **milliseconds**, from the transcript.
    ///
    /// ⚠ **Not `started`, and the difference is the whole point.** `started` is
    /// when this console picked the process up; this is when the conversation
    /// last moved. For a session running since last night they are thirteen hours
    /// apart, and the second one is what somebody scanning the list wants. Filled
    /// by the roster, which reads the file — see [`crate::past::touched`]. Absent
    /// for a session whose transcript cannot be found, so a client can leave the
    /// column empty rather than print the epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touched: Option<u64>,
    /// How much the transcript weighs, in bytes — the whole conversation as it
    /// stands on disk. Filled by the roster from the same metadata read as
    /// [`Self::touched`]; see [`crate::past::sized`] for why this is not the
    /// same fact as [`Self::context`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub alive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// What the CLI last said it was doing, when it is doing anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy: Option<String>,
    /// Whether a turn is running — observed by the runner, not narrated by the
    /// CLI.
    ///
    /// ⚠ **[`Self::busy`] cannot answer this and reading it as though it could
    /// called a working session idle.** A status is announced when it *changes*,
    /// so a long stretch of one activity, or one the CLI does not narrate, leaves
    /// nothing standing — and no status was drawn as *idle*. Reported from the
    /// phone 2026-08-07 about a session that was running tools throughout
    /// (memview #112), and it made #111's invisible queue actively misleading:
    /// a message sent to a session the page calls idle should land at once, so
    /// its not landing reads as a failure.
    ///
    /// A turn ending is an event the runner sees; so is the traffic while one
    /// runs. This is those, and nothing the CLI has to be asked for. Deliberately
    /// **not** a timeout over the last status — the console has had two defects
    /// from inferring state on a timer, and a turn can legitimately be quiet for
    /// minutes.
    pub working: bool,
    /// How many times someone has spoken to this session since it was last
    /// compacted — exchanges, not messages, and not the result line's
    /// `num_turns`. See [`crate::past::counted`] for why it is counted from
    /// the transcript rather than added up as turns arrive.
    pub interactions: u32,
    /// What this session's tokens would have cost at API list prices.
    ///
    /// ⚠ **This is not money.** A session inherits the CLI's own credentials and
    /// runs on the subscription, so nothing here is billed per token — it is a
    /// weight wearing a currency symbol, and shown as one it reads as a bill.
    /// The client shows it only when [`Self::limit`] says the account has
    /// stopped being all-you-can-eat, which is the first moment it means
    /// anything.
    pub cost_usd: f64,
    /// How many tokens the last request's prompt came to, and the window it went
    /// into — so a reader can see when compaction is coming rather than meeting it.
    ///
    /// ⚠ Fullness is per MESSAGE, not per turn. The result line carries a usage
    /// too and it sums every request the turn made: a turn of 23 requests read
    /// 1.6M against a 1M window. Shipped that, saw it on the phone, fixed it.
    ///
    /// ⚠ Prompt size is input + cache-creation + cache-read added together. The
    /// cached part is almost all of it — 496,000 read against 2 of input on this
    /// session — so anything reading `input_tokens` alone reports nearly zero for
    /// a conversation that is nearly full.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
    /// How many background tool calls this session has started and not had
    /// reported finished.
    ///
    /// ⚠ **Only the ones the harness tracks.** A command backgrounded inside a
    /// shell — `nohup … &` — returns at once and announces nothing, so it is
    /// invisible here. This counts what can be seen, and the client's wording
    /// claims no more than that.
    ///
    /// Counted by the runner rather than by whoever is watching, because the
    /// list is drawn without opening anything: the page that knew this before
    /// was the session's own, from its event stream, so the list could not say
    /// it at all.
    #[serde(skip_serializing_if = "none")]
    pub background: usize,
    /// WHICH background calls are still running, not just how many.
    ///
    /// ⚠ **`background` is kept beside this deliberately.** The list ranks a row
    /// on whether anything is running and never draws the names, so it wants a
    /// number; the session strip wants the name, because *1* is only a reason to
    /// ask (memview #740). Same fact, two readers, and deriving the count from
    /// this vector in the client would put the ranking at the mercy of a label.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running: Vec<crate::protocol::Called>,
    /// The account's own verdict on its rate limit, when it has given one:
    /// `allowed`, `allowed_warning` or `rejected`.
    ///
    /// The CLI's vocabulary, read off the 2.1.220 binary rather than guessed.
    /// `None` until the account says something, which is the common case — and
    /// the reason cost is hidden by default: no news is not news of trouble.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    /// The first thing this session was asked to do, kept as its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asked: Option<String>,
    /// What the conversation calls itself — `memview`, `health`. Filled in by
    /// the roster from the transcript, because the session's own process never
    /// says it. See [`crate::past::named`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What the session may do without asking: `default`, `plan`, `dontAsk`,
    /// `acceptEdits`, `auto`, `bypassPermissions`.
    ///
    /// ⚠ **This is what the console set, not what the transcript says.** The
    /// first version of this read the last `permission-mode` line from the file,
    /// which is wrong for the case that matters: a session *resumed* from an
    /// interactive one carries that session's mode lines, so the header reported
    /// `Auto` over a console that had passed no mode at all and was asking
    /// permission for every single call. The console is the only thing that
    /// knows what it asked for.
    ///
    /// **Stored names are not the displayed ones** — `default` is shown as
    /// *Manual* — so the client keeps the CLI's own label table rather than
    /// prettifying these itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Why the last mode change was refused, in the CLI's own words.
    ///
    /// ⚠ **Present only until the next change is asked for**, because it
    /// describes an attempt rather than a state. [`mode`](Self::mode) beside it
    /// has already been put back to what the session is actually in, so this is
    /// the explanation for a switch that appeared to happen and then did not.
    ///
    /// The CLI's wording rather than this console's: measured on 2026-08-16, it
    /// says *"Cannot set permission mode to bypassPermissions because the
    /// session was not launched with --dangerously-skip-permissions"*, which
    /// names the cause and the remedy better than anything written from here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_refused: Option<String>,
    /// How many questions it is blocked on. The one number that means "this
    /// session cannot go on without you", so it belongs in the list of sessions
    /// and not only on the page of one.
    pub waiting: usize,
    /// How many messages have been written to this session and not read back.
    pub unread: usize,
    /// How long it has been failing to read them, in **seconds** — present only
    /// when the console is prepared to call it deaf. See [`Session::deaf`].
    ///
    /// Seconds, not milliseconds: this is a duration somebody reads off a card
    /// to decide whether to restart a session, and the millisecond it began is
    /// not a fact anybody wants at that moment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deaf: Option<u64>,
    /// Slash commands waiting for the turn to end, oldest first. See
    /// [`State::held`] for why they are not simply written.
    ///
    /// The words themselves, because the client draws them and cancels by them:
    /// what is on screen has to say WHICH command is waiting, or it is one more
    /// thing happening that nobody was told about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<String>,
}

/// An event and its place in the session's order.
///
/// The number is what makes a dropped connection survivable. Without it a
/// reconnecting client has no way to say what it already has, so the only safe
/// thing the server can do is send everything again and the only safe thing the
/// client can do is throw its page away — which on a phone, over a tunnel, meant
/// losing the history somebody had just scrolled back to read because a train
/// went through a cutting.
///
/// Sequential from 1, per session, assigned under the same lock that appends to
/// the log — so the number a client holds names exactly one event and the ones
/// after it are exactly what it missed.
#[derive(Debug, Clone)]
pub struct Stamped {
    pub seq: u64,
    /// When it happened, in milliseconds since the epoch. See [`protocol::Timed`]
    /// — a live event is stamped as it arrives, a replayed one carries what the
    /// transcript recorded, and a transcript line need not have said.
    pub at: Option<i64>,
    pub event: Event,
}

/// Nothing to report, for a count that is left off the wire when it is zero.
///
/// Absent rather than `0` so that a client can ask "is anything running" of the
/// field's presence, and so an older client sees the same shape it always did.
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
    /// False means the client's page cannot be kept: it named nothing, or named
    /// an event this session no longer holds.
    pub resumed: bool,
    pub events: Vec<Stamped>,
    /// The highest sequence number this client has now been sent — including the
    /// one it arrived holding, when there was nothing after it. Live events at or
    /// below it are duplicates and must not be sent again.
    pub through: u64,
}

/// Whether a client holding everything through `after` can be sent only what it
/// missed, given a log holding `held_from..=issued`.
///
/// Two things disqualify a number, and both mean the same thing — the events
/// between what the client has and what the log holds cannot be produced:
///
/// * **Older than the log's front.** The session ran on past this client's place
///   while it was away and the scrollback dropped the difference. A gap here
///   would be silent, which is worse than the visible cost of starting again.
/// * **Newer than anything issued.** The client is quoting another session's
///   numbering — a console restarted under the same id — and resuming it would
///   hide every real event until the count caught back up.
///
/// `after + 1` rather than `after` on the left: what has to still be reachable is
/// the *next* event, not the one already held. That makes an exactly-caught-up
/// client resumable against an empty log, which is the common case — somebody
/// reconnecting to a session that has said nothing since.
pub fn resumable(after: u64, held_from: u64, issued: u64) -> bool {
    after + 1 >= held_from && after <= issued
}

/// A question the session is waiting on an answer to.
///
/// The arguments are kept because an allow has to echo them back, and the tool's
/// name because only one tool's arguments may be *edited* on the way back — see
/// [`protocol::QUESTION_TOOL`]. The CLI's own sentence is kept for the same
/// reason the event carries it: it reads better than one reassembled here.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    pub tool: String,
    /// The call being asked about — the `tool_use` id. Carried across an upgrade
    /// with the rest, or a re-seeded question would come back unable to say
    /// which tool row it belongs to and would draw a second widget beside it.
    /// See [`crate::protocol::Event::Ask`].
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
    /// The last sequence number issued. Not the log's length: the log is a
    /// scrollback and drops its front, and a number that was reused after an
    /// eviction would resume a client into the wrong place.
    issued: u64,
    /// Questions the session is blocked on, by control-request id.
    pending: BTreeMap<String, Pending>,
    alive: bool,
    /// When the kill armed by [`Session::stop`] falls due, in epoch
    /// milliseconds, and `None` for a session nobody has stopped. Read by
    /// [`crate::roster::Roster::handover`], which is the only reason it is
    /// written down rather than left to the timer — see [`Session::stop`].
    stopping: Option<i64>,
    model: Option<String>,
    busy: Option<String>,
    /// See [`Summary::interactions`], and [`crate::past::counted`] for why the
    /// byte offset travels with the number.
    counted: crate::past::Counted,
    /// See [`Summary::mode`]. Set when the session is spawned, written
    /// optimistically when the console asks for a change, and **corrected when
    /// the CLI answers** — see [`Session::settle_mode`]. Carried across an
    /// upgrade, because no file records it in a way that can be trusted.
    mode: Option<String>,
    /// What the mode was before a change the CLI has not answered yet, so a
    /// refusal can put back the mode the session is actually in.
    ///
    /// `None` means nothing is outstanding. Cleared either way when the answer
    /// arrives, so a later refusal cannot restore a mode from two changes ago.
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
    /// Background tool calls started and not yet ended, by the id of the call
    /// that started each, against the task id the harness gave it. See
    /// [`Summary::background`].
    ///
    /// Keyed by the call because that is what a notification names; carrying the
    /// task because that is what a kill names, and a kill is silent afterwards.
    background: std::collections::BTreeMap<String, crate::protocol::Called>,
    /// The last few tool calls seen, by call id, so a background one can be
    /// NAMED when its result arrives.
    ///
    /// ⚠ **A ring, not a map that grows.** The `Tool` event naming a call
    /// immediately precedes the `ToolResult` that detaches it, so only the
    /// recent ones can ever be needed — and an unbounded map here would hold
    /// every call of a session that runs for days.
    called: std::collections::VecDeque<(String, crate::protocol::Called)>,
    /// When the process last wrote a line, in epoch milliseconds. See
    /// [`Session::heard`]; zero for one that has never said anything.
    heard: i64,
    asked: Option<String>,
    stderr: String,
    /// Messages written to stdin that the CLI has not echoed back, oldest
    /// first. See [`Session::deaf`].
    unread: VecDeque<Unread>,
    /// Whether the session has spoken since its last turn ended — the whole of
    /// what [`Summary::working`] reports.
    ///
    /// ⚠ **A positive fact, not the absence of a negative one.** This was first
    /// derived from `idle_since.is_none()`, which is true for a session that has
    /// said *nothing at all* — so a freshly started session, still loading and
    /// with no `Started` line on the wire yet, was reported as working. Seen on
    /// screen within a minute of shipping it.
    working: bool,
    /// When the last turn ended, in epoch milliseconds — `None` whenever the
    /// session is working. See [`Session::deaf`] for why this and not silence.
    idle_since: Option<i64>,
    /// Whether this episode of deafness has already been announced. See
    /// [`Session::check_deaf`].
    announced_deaf: bool,
    /// When the oldest decision the session has not acted on was written, in
    /// epoch milliseconds. See [`Session::deaf`].
    decided: Option<i64>,
    /// Slash commands written while a turn was running, oldest first, waiting
    /// for it to end.
    ///
    /// ⚠ **A command sent mid-turn does not run — it is handed to the MODEL as
    /// words.** The CLI parks it as a `queued_command` with
    /// `commandMode: "prompt"` (1,756 of them on this machine) and releases it
    /// into the conversation when the turn ends, so `/rename` reached an agent
    /// which replied "nothing for me to do" while no name was ever written.
    /// Nothing on any screen said the command had been demoted.
    ///
    /// Held here rather than in the client, because a client that is holding it
    /// stops holding it the moment the phone is put away — and this console's
    /// sessions are usually working, so the demoted case is the common one, not
    /// the edge. See [`Session::send`] and [`Session::release_held`].
    held: VecDeque<String>,
    /// A `/compact` has been sent and the conversation has not moved since.
    ///
    /// The one long silence that is not a fault: a compaction summarises the
    /// whole history before anything else happens, and measured on `hardware`
    /// 2026-08-08 it left the transcript frozen for minutes. See
    /// [`Session::deaf`].
    compacting: bool,
}

/// How long a message may sit unread, between turns and with the session
/// otherwise silent, before the console stops calling it *waiting* and calls the
/// session deaf.
///
/// ⚠ **Bounded by evidence at both ends.** The legitimate wait this has to clear
/// is a message that arrives just as a turn ends, which is seconds — the long
/// waits measured on 2026-08-07, up to twelve minutes for the oldest of four,
/// were input parked *mid-turn*, and a working session never reaches this test
/// at all because [`State::idle_since`] is unset while it works. The failures
/// this has to catch were both silent for over twenty minutes. Ninety seconds
/// sits an order of magnitude clear of each.
const DEAF_AFTER_MS: i64 = 90_000;

/// The same wait, while a compaction is outstanding.
///
/// ⚠ **A compaction is a legitimate silence with no pulse at all.** Measured on
/// `hardware` 2026-08-08: `/compact` sent at 09:50:46, and twenty seconds later
/// the transcript was still frozen where it had been at 09:49:53 — it stays that
/// way for minutes while a 437k-token context is summarised, so neither the file
/// nor the process says anything a shorter wait could tell apart from deafness.
///
/// Longer rather than suppressed outright, because a session can go deaf *around*
/// a compaction — one of the two episodes this task is named for did — and an
/// alarm that a single command can switch off for ever is worth less than the
/// wolf it might cry.
const DEAF_AFTER_COMPACT_MS: i64 = 15 * 60_000;

/// Drop one piece of background work from the count, by whichever name the
/// thing that ended it knew.
///
/// Both ways in — the live stream and the reread of the file — end work by both
/// names, so the branch lives here rather than twice. A removal for a call
/// because the map is keyed by them; a search for a task because it is carried
/// on the value, and a monitor that timed out has no other name to give.
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

/// Keep track of what is in flight, and of whether the session is in a position
/// to read it.
///
/// Everything [`Session::deaf`] decides on is maintained here, in one place,
/// because the verdict is a conjunction and a field updated in only some of the
/// arms that should update it fails silently — as an alarm that never fires.
fn in_flight(state: &mut State, event: &Event) {
    match event {
        // The read receipt. Oldest match first, for the same reason the client
        // promotes the oldest waiting entry: stdin is a queue, and the same
        // words sent twice must be answered in the order they were written.
        Event::Prompt { text } => {
            if let Some(at) = state.unread.iter().position(|held| &held.text == text) {
                state.unread.remove(at);
            }
            // It read something, so whatever this episode was, it is over — and
            // if it happens again it is worth saying again.
            state.announced_deaf = false;
        }
        // A turn ended, so from here the session owes us a read.
        Event::Turn { .. } => state.idle_since = Some(now()),
        // Nothing is running as of now.
        //
        // ⚠ **Both, and `Joined` matters most.** It is pushed *after* the seeded
        // transcript, so it is what stops a conversation whose file ends
        // mid-turn — killed, crashed, compacted — from reading as a turn that is
        // still going in a process that has only just started. `Started` covers
        // the fresh spawn, which has never had a turn to end.
        Event::Started { .. } | Event::Joined { .. } => state.idle_since = Some(now()),
        _ => {}
    }
    state.working = working_after(state.working, event);
    match event {
        Event::Command { text } if text.starts_with("/compact") => state.compacting = true,
        // A decision written down the pipe of a session that ASKED for it and is
        // blocked until it arrives.
        //
        // ⚠ **Its own clock, needing no `idle_since`.** A session blocked on a
        // question is mid-turn, so the message test above can never fire for it —
        // which is why `health` sat on an answered question for thirty-one
        // minutes with nothing on screen but a green tick (memview #122). Here
        // there is no ambiguity to allow for: the session said it could go no
        // further without this, so silence afterwards is not work.
        Event::Answered { .. } => state.decided = state.decided.or(Some(now())),
        _ => {}
    }
    // Anything the session says of its own accord means it is working, and a
    // working session is not deaf however long it has been quiet. Deliberately
    // NOT `Busy`: a status is announced only when it changes (memview #112), so
    // its absence says nothing and its presence can be minutes old.
    //
    // A `Turn` counts here too — it is the session speaking — but it must not
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

/// Whether a turn is running, after `event`.
///
/// Set by the session speaking, cleared when the turn ends — and false until it
/// has ever spoken, which is what a session that is still starting up actually
/// is.
///
/// ⚠ **`Started` and `Joined` clear it**, for exactly the reason they set
/// `idle_since` in [`in_flight`]: `Joined` is pushed after the seeded
/// transcript, so it is what stops a conversation whose file ends mid-turn —
/// killed, crashed, compacted — from reading as a turn still running in a
/// process that has only just started. That half was written for `idle_since`
/// and not for this, so a resumed session could be marked idle and working at
/// once. Measured: `hardware` resumed 2026-08-08 22:53 and its card read
/// `working` for 84 minutes over a process with no API socket, a flat 0.5% of a
/// core and nothing appended to its transcript since that morning; a message
/// sent at 00:17 was picked up at once. See memview #640.
///
/// Public for the reason [`deaf_after`] is: this is the part worth testing, and
/// reaching the case that was wrong otherwise needs a transcript ending mid-turn
/// and a resume to read it.
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
        // Everything else says nothing either way — a status, a decision, a
        // command — and must leave the answer where it was.
        _ => was,
    }
}

/// The verdict itself, taken against state a caller is already holding — see
/// [`Session::deaf`], which is this with the lock taken and the documentation.
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

/// The verdict as arithmetic, apart from where its inputs come from.
///
/// Public because it is the part worth testing: the conjunction, and which of
/// the two clocks the wait is measured from. Waiting ninety seconds in a test to
/// find out would make it a test nobody runs.
///
/// * `idle_since` — when the last turn ended, `None` while the session works.
/// * `oldest` — when the oldest unread message was written, `None` for none.
/// * `decided` — when the oldest unacted-on decision was written, `None` for
///   none. See [`Session::deaf`] for why this one needs no `idle_since`.
///
/// **Two ways to be waiting, and either is enough.** They are not variants of
/// one test: a message needs the session to be between turns before its silence
/// means anything, and a decision does not, because the session asked for it and
/// stopped. Whichever has waited longer is the one reported.
pub fn deaf_after(
    idle_since: Option<i64>,
    oldest: Option<i64>,
    decided: Option<i64>,
    compacting: bool,
    now: i64,
) -> Option<i64> {
    // The LATER of the two: before the turn ended the session was entitled to
    // park the message, and before the message arrived there was nothing to
    // read. Only after both has it been given the chance this measures.
    let unread_since = idle_since.zip(oldest).map(|(idle, at)| idle.max(at));
    // The EARLIER of the two cases, so a long wait is not hidden by a short one
    // that started later.
    let since = [unread_since, decided].into_iter().flatten().min()?;
    let allowed = if compacting {
        DEAF_AFTER_COMPACT_MS
    } else {
        DEAF_AFTER_MS
    };
    let waited = now - since;
    (waited >= allowed).then_some(waited)
}

/// A message written to the session's stdin that it has not read back.
///
/// The pair of it is what makes deafness observable at all:
/// [`Event::Accepted`] says the bytes reached the pipe and the CLI's replay
/// says they were taken out of it, so an entry that sits here is a message in
/// flight and nothing else. Commands are deliberately absent — the CLI does not
/// replay one, so a command would sit here for ever. See [`Event::Command`].
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
    /// The process id, kept because an adopted session has no [`Child`] handle
    /// to kill through — the handle belonged to the image that exec'd away.
    pid: u32,
    /// See [`Fds`]. Kept for the same reason.
    fds: Fds,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    tx: broadcast::Sender<Stamped>,
}

/// By hand because the sink is a trait object, which cannot derive it — and the
/// interesting half is the identity anyway.
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
    /// What the session may do without being asked.
    ///
    /// This matters more than it looks. In headless mode there is nobody to
    /// answer a permission prompt, so under the CLI's default mode **every tool
    /// call that needs permission is refused** — measured, not assumed: a
    /// `Write` in a fresh session came back `is_error` with no file created. A
    /// console left on the default is therefore a console that can converse and
    /// nothing else, until the approval channel of phase 2 exists.
    pub permission_mode: Option<String>,
}

impl Session {
    /// Start a session in `dir`, with `id` as both our handle and its session id.
    pub fn start(id: String, dir: &Path, spawn: &Spawn) -> Result<Arc<Self>> {
        Self::spawn(id, dir, spawn, false)
    }

    /// Pick up a conversation that already exists, keeping its id.
    ///
    /// `--resume` rather than `--session-id`: the two are alternatives, and
    /// passing an id the CLI has never seen to `--resume` is an error rather than
    /// a fresh start, which is the behaviour worth having — a typo should not
    /// silently open an empty session wearing the name of a real one.
    ///
    /// ⚠ **The transcript is not a lock.** Nothing stops two processes resuming
    /// the same id and both appending; the roster refuses a second *console*
    /// session, but a `claude` in a terminal is invisible to it. So this is for a
    /// conversation that has been closed, and the console cannot check that for
    /// you.
    pub fn resume(id: String, dir: &Path, spawn: &Spawn) -> Result<Arc<Self>> {
        // ⚠ **Nothing is recorded here about the file's date, and there was.**
        // Picking a conversation up writes to its transcript — `mode`,
        // `permission-mode` and `bridge-session` lines go in at the moment of
        // resume — so this used to read the file's date and size first and keep
        // them as a floor. That worked only while nothing was appended, and
        // those three lines are appended: `scanner`, opened after two days, said
        // `just now`. The date now comes out of the conversation itself; see
        // [`crate::past::last_moved`].
        let session = Self::spawn(id, dir, spawn, true)?;
        session.seed();
        Ok(session)
    }

    /// Put what was already said in front of what happens next.
    ///
    /// ⚠ **`--resume` restores the CLI's context, not the console's view.** The
    /// process comes back knowing the whole conversation and replays none of it on
    /// stdout, so without this a resumed session opens empty and its turn count
    /// reads `0` — the console's count of what it watched, which a person reads as
    /// the conversation's length. It looked like resume had not worked.
    ///
    /// The transcript on disk is the same vocabulary the stream uses, so the fix
    /// is a different reader over the same shapes rather than a second model of a
    /// conversation. See [`crate::protocol::read_recorded`].
    ///
    /// Silent when there is nothing to find. A conversation with no transcript we
    /// can locate still resumes — the CLI has its own copy — and an empty view is
    /// what it was before this existed.
    fn seed(self: &Arc<Self>) {
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
        // Last, so it sits between what was read and what we watch — and it
        // carries the cursor, which is the only thing that knows where this page
        // began. A client asking for what came before has nothing else to go on.
        // Stamped now, because joining is the one thing here that did happen now.
        self.push(Event::Joined {
            earlier: count,
            from: seed.from,
        });
        self.recount();
    }

    /// Count the exchanges the transcript has gained. See [`crate::past::counted`].
    ///
    /// Silent when there is no transcript yet — a session that has just been
    /// started has none, and reporting zero for it is right anyway. Reads the
    /// file outside the lock, which is the whole reason this is a method and not
    /// a line in `push_at`: this is the only thing in the session that touches a
    /// file, and the state lock is taken by every event that arrives.
    ///
    /// Safe to call from anywhere in the reading task and nowhere else — it
    /// reads the offset, then the file, then writes both back, and nothing else
    /// in this console writes that pair.
    fn recount(&self) {
        let root = crate::past::projects_root();
        let Some(path) = crate::past::transcript_of(&root, &self.id) else {
            return;
        };
        let mut so_far = self.state.lock().expect("session state poisoned").counted;
        // A seed arrives here at zero, and zero is the whole file — 1.08 GB and
        // 3.3 seconds for the largest conversation on this machine, on the
        // executor, inside the handler that answers "resume this one". The count
        // it arrives at was decided by the last megabyte, so start where that
        // begins. See [`crate::past::seed_from`] for why the two agree exactly.
        if so_far.through == 0 {
            so_far.through = crate::past::seed_from(&path);
        }
        let found = crate::past::counted(&path, so_far);
        let mut state = self.state.lock().expect("session state poisoned");
        state.counted = found.counted;
        // The other half of what that read found: work the harness has reported
        // finished. It closes the count here rather than through an event,
        // because there is no event — see [`crate::past::Appended::finished`].
        for named in &found.finished {
            forget(&mut state.background, named);
        }
        // ⚠ **A compaction is announced in the file and nowhere else**, so this
        // read is the only way a running session learns that its own fullness
        // describes a conversation it no longer holds. Taken from the file
        // rather than simply cleared, because the same few kilobytes may carry a
        // request made after the boundary — and then the new figure is already
        // known and there is no reason to show nothing.
        if found.compacted {
            state.context = found.context;
        }
    }

    fn spawn(id: String, dir: &Path, spawn: &Spawn, resuming: bool) -> Result<Arc<Self>> {
        let mut command = Command::new(&spawn.binary);
        command
            .current_dir(dir)
            .arg("-p")
            // stream-json output is refused without --verbose, which is a CLI
            // validation rule rather than a preference of ours.
            .arg("--verbose")
            .args(["--input-format", "stream-json"])
            .args(["--output-format", "stream-json"])
            .arg("--include-partial-messages")
            // The echo of our own prompt is how a client knows the message
            // landed; see `protocol::Event::Prompt`.
            .arg("--replay-user-messages")
            .args(if resuming {
                ["--resume", &id]
            } else {
                ["--session-id", &id]
            })
            // **The switch that makes approvals possible at all.** Undocumented
            // in `--help` at 2.1.220 and found by reading the TypeScript SDK,
            // which passes exactly this: without it a session in `manual` mode
            // refuses every tool call outright and reports it in
            // `permission_denials`, and no question ever reaches the client.
            // With it, the CLI asks over the same stream it answers on.
            .args(["--permission-prompt-tool", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Without this the child keeps running when the console is killed,
            // holding the session id and the working directory.
            .kill_on_drop(true);
        if let Some(model) = &spawn.model {
            command.args(["--model", model]);
        }
        if let Some(mode) = &spawn.permission_mode {
            command.args(["--permission-mode", mode]);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning {} in {}", spawn.binary, dir.display()))?;
        // Read before the handles are moved out: after the upgrade these numbers
        // are all that is left of the connection to this process.
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
                // What was actually asked for. **Unset is not unknown** — it is
                // the CLI's own default, under which every tool call needing
                // permission comes back here for an answer. Recording it as
                // `default` says that plainly instead of leaving the header
                // blank about the one setting that governs every tap.
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

    /// Take over a session the previous image was running.
    ///
    /// Everything about the process is unchanged — same pid, same pipes, same
    /// conversation — because `execve` replaced this console's image without
    /// touching its children. What is lost is the [`Child`] handle, so exit is
    /// noticed by the child's stdout reaching end of file rather than by waiting
    /// on it, and killing goes through the pid.
    ///
    /// ⚠ The scrollback does not survive. A client that reconnects is reseeded
    /// from the transcript on disk, which is the durable record anyway — except
    /// for [`Tally`], which no transcript holds and which therefore has to be
    /// carried.
    pub fn adopt(
        id: String,
        dir: PathBuf,
        pid: u32,
        fds: Fds,
        mut tally: Tally,
    ) -> Result<Arc<Self>> {
        let pending = std::mem::take(&mut tally.pending);
        // ⚠ The scrollback did NOT survive the exec, and a client reconnecting
        // to an empty log is told its page is unresumable and starts blank. So
        // an adopted session reseeds from the transcript exactly as a resumed
        // one does — the conversation is on disk either way, and losing it on
        // an *upgrade* would make the upgrade worse than the restart it
        // replaced. Shipped without this and the history vanished; see
        // [`Self::seed`].
        // SAFETY: these descriptors were handed over by the image that exec'd,
        // which held them open and cleared close-on-exec so they would survive.
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
            // A tally from an image that never counted a turn has a zero here;
            // now is the only honest answer in that case.
            started: match tally.started {
                0 => SystemTime::now(),
                secs => UNIX_EPOCH + Duration::from_secs(secs),
            },
            state: Mutex::new(State {
                alive: true,
                model: tally.model,
                mode: tally.mode,
                cost_usd: tally.cost_usd,
                window: tally.window,
                limit: tally.limit,
                // Carried like the rest of the tally: nothing on disk records
                // it, so an upgrade that dropped it would blank the front
                // page until the next request came back.
                spent: tally.spent,
                counted: tally.counted,
                // The turn that was in flight is still in flight: `execve` does
                // not touch the child, so whatever it was doing it is still
                // doing, and its own next status line or `Turn` will correct
                // this the moment one arrives.
                busy: tally.busy,
                ..State::default()
            }),
            stdin: tokio::sync::Mutex::new(Some(Box::new(stdin) as Sink)),
            pid,
            fds,
            kill: Mutex::new(Some(kill_tx)),
            tx,
        });
        session.seed();
        // **After the seed, so the question lands where it happened: at the end
        // of the conversation, which is where it is still standing.** Pushed as
        // an ordinary `Ask` rather than restored into `pending` directly,
        // because that is the same path a live question takes — one mechanism,
        // so a client that reconnects is offered the decision again *and* the
        // session is recorded as waiting for it, from a single event.
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
        session.clone().read_from(Some(stdout), Some(stderr), true);
        reap_adopted(pid);
        Ok(session)
    }

    /// What this session has counted, for an upgrade to hand on. See [`Tally`].
    pub fn tally(&self) -> Tally {
        let state = self.state.lock().expect("session state poisoned");
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
            busy: state.busy.clone(),
            pending: state.pending.clone(),
            counted: state.counted,
        }
    }

    /// The pipes to this session's process, for an upgrade to hand on.
    pub fn fds(&self) -> Fds {
        self.fds
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Read the child's streams until they end.
    ///
    /// ⚠ **`ends_on_eof` decides who declares the session over**, and it is not
    /// a preference. For an adopted session end of file is the only signal there
    /// is — no [`Child`] survived the upgrade to be waited on. For a spawned one
    /// [`Self::reap`] must be the one to say so, because **only it knows the exit
    /// code**: letting the reader win a race it usually wins turned every clean
    /// exit into `code: None`, which reads as "killed". A test caught that.
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
                // Delay, not Burst: a session that was busy for a minute owes us
                // one catch-up read, not a minute's worth back to back.
                beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    let line = tokio::select! {
                        line = lines.next_line() => line,
                        // ⚠ **The transcript changes when the process says
                        // nothing.** A compaction is written to the file and
                        // announced on no stream, so tying the read to `Turn`
                        // meant a session that compacted and then sat waiting
                        // for its next instruction never read its own boundary:
                        // `home` showed 258,318 tokens for ninety minutes, which
                        // was the fullness of a conversation that had stopped
                        // existing, and 13 exchanges the boundary had reset to 0.
                        // A stale figure and a live one are drawn identically.
                        _ = beat.tick() => {
                            session.recount();
                            continue;
                        }
                    };
                    let Ok(Some(line)) = line else { break };
                    // ⚠ **Every line, whatever it turns out to be.** This is the
                    // record of when the *process* last spoke, which is how the
                    // roster decides who to ask for the account's usage — and an
                    // idle session answers that question from a cache as old as
                    // its own last request. See [`Session::heard`].
                    session.heard();
                    // Before the events, and separately from them: a control
                    // response is an answer to something the console asked, not
                    // something that happened in the conversation, so it has no
                    // place in a transcript anyone reads. See
                    // [`protocol::usage_reply`].
                    if let Some(windows) = protocol::usage_reply(&line) {
                        session.record_usage(windows);
                        continue;
                    }
                    // The other answer this console asks for, and for a long
                    // time the one nothing read — see [`Session::settle_mode`].
                    if let Some(reply) = protocol::mode_reply(&line) {
                        session.settle_mode(reply);
                        continue;
                    }
                    for event in protocol::read(&line) {
                        // The end of a turn is the one moment the exchange count
                        // can have changed, and by then the CLI has written the
                        // whole exchange to its transcript. Recounted rather than
                        // incremented — see [`crate::past::counted`] — and
                        // done here rather than in `push_at`, which holds the
                        // state lock and must not be reading files. The heartbeat
                        // above does not replace this: at the end of a turn the
                        // count is wanted NOW, not within [`RECOUNT_EVERY`].
                        let counted = matches!(event, Event::Turn { .. });
                        session.push(event);
                        if counted {
                            session.recount();
                            // The moment the commands parked mid-turn have been
                            // waiting for. Here rather than in `push_at` for the
                            // same reason as the recount: that one holds the
                            // state lock, and this writes to a pipe. A failure
                            // is logged and not propagated — this loop is the
                            // session's only reader, and ending it over a
                            // refused write would take the transcript with it.
                            if let Err(err) = session.release_held().await {
                                tracing::warn!("{}: holding a command back: {err:#}", session.id);
                            }
                        }
                    }
                }
                // The pipe closed, so the process did — but only say so when
                // nothing better is watching. See the note above.
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

    /// Wait for a spawned child, and kill it when asked.
    ///
    /// The adopted half of this is [`reap_adopted`], which has only a pid to
    /// work with.
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

    /// Record that the process is gone, once.
    ///
    /// Called from two places for a spawned session — the reader seeing end of
    /// file and the reaper seeing the exit — and from one for an adopted one.
    /// Guarded so a client is not told twice that the same session ended.
    fn ended(&self, code: Option<i32>) {
        {
            let mut state = self.state.lock().expect("session state poisoned");
            if !state.alive {
                return;
            }
            state.alive = false;
        }
        self.push(Event::Exited { code });
    }

    /// Send a message to the session.
    ///
    /// ⚠ **A slash command sent mid-turn is HELD, not written.** See
    /// [`State::held`] for what the CLI does with one instead. The test and the
    /// parking happen under a single lock on the state, which is the same lock
    /// [`in_flight`] takes to clear `working` — so a turn ending beside this
    /// either has not happened yet, and the flush that follows it drains what
    /// was just parked, or has happened, and this writes straight through.
    pub async fn send(&self, text: &str) -> Result<()> {
        let parked = {
            let mut state = self.state.lock().expect("session state poisoned");
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
        // ⚠ **Announced on the way in, not on the echo.** The write above has
        // succeeded, so the message is the CLI's problem now — but the CLI may
        // not read it for minutes, and until it does nothing else on the wire
        // mentions it. See [`Event::Accepted`] for the measurements.
        //
        // Which of the two it is has to be decided here, because it is a
        // statement about what will come back and only the text can say: a
        // prompt is echoed by `--replay-user-messages` and a command is not.
        // See [`Event::Command`].
        if protocol::is_command(text) {
            self.push(Event::Command {
                text: text.to_string(),
            });
        } else {
            // In flight until the CLI replays it, which is the whole of what
            // [`Self::deaf`] has to go on.
            self.state
                .lock()
                .expect("session state poisoned")
                .unread
                .push_back(Unread {
                    text: text.to_string(),
                    at: now(),
                });
            self.push(Event::Accepted {
                text: text.to_string(),
            });
        }
        // Held even if the CLI never echoes it, so the record of what was asked
        // does not depend on the CLI's replay behaviour.
        let mut state = self.state.lock().expect("session state poisoned");
        if state.asked.is_none() {
            state.asked = Some(text.to_string());
        }
        Ok(())
    }

    /// Write the commands that were waiting for this turn to end.
    ///
    /// Through [`Self::send`], which finds `working` already false and writes
    /// straight through — so a released command takes the ordinary path and is
    /// recorded by the ordinary [`Event::Command`], at the moment it actually
    /// goes. One at a time, and re-locked between each, so a cancel arriving
    /// mid-drain is honoured rather than raced.
    ///
    /// A write that fails stops the drain and leaves the rest held: the usual
    /// reason is a session that has stopped taking input, and writing the second
    /// command after the first was refused would be pretending.
    pub async fn release_held(&self) -> Result<()> {
        loop {
            let next = {
                let mut state = self.state.lock().expect("session state poisoned");
                state.held.pop_front()
            };
            let Some(command) = next else { return Ok(()) };
            self.send(&command).await?;
        }
    }

    /// Take back a command that is waiting — by its exact text, which is what
    /// the client has.
    ///
    /// Returns whether anything was holding it. A false is not an error: two
    /// screens can be looking at the same session, and the second tap on a
    /// command already released has nothing to undo.
    pub fn forget_held(&self, text: &str) -> bool {
        let mut state = self.state.lock().expect("session state poisoned");
        let Some(at) = state.held.iter().position(|held| held == text) else {
            return false;
        };
        state.held.remove(at);
        true
    }

    /// Show the session a picture, with whatever was said about it.
    ///
    /// The same write as [`Self::send`] and deliberately not folded into it: the
    /// two differ in what they put on the wire (see
    /// [`protocol::prompt_with_image`]), and an `Option<Image>` on the ordinary
    /// send would put a branch on the path every message in the console takes.
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
        let mut state = self.state.lock().expect("session state poisoned");
        if state.asked.is_none() {
            // What it was opened for, when a picture is the first thing said. The
            // base64 is emphatically not this: it is a megabyte of characters, and
            // this is a line on the front page.
            state.asked = Some(match text.trim() {
                "" => "an image".to_string(),
                words => words.to_string(),
            });
        }
        Ok(())
    }

    /// Answer a question the session is blocked on.
    ///
    /// Refusing carries a reason, because the session is told it and can act on
    /// it — "not now, do the read-only part first" is a useful thing to say to
    /// an agent, and a bare denial is not.
    ///
    /// Answering an unknown id is an error rather than a silent success: the
    /// likeliest cause is two people looking at the same session, and the second
    /// one deserves to be told that the decision was already taken.
    ///
    /// `reply` is what was said about a [`protocol::QUESTION_TOOL`] call, and is
    /// refused for anything else. That is the narrow reading of `updatedInput`:
    /// the protocol would let a client rewrite the arguments of any tool it
    /// approves, and a console whose whole job is approving tool calls should not
    /// also be able to change what it approved.
    pub async fn decide(
        &self,
        id: &str,
        allowed: bool,
        why: &str,
        reply: Option<&protocol::Reply>,
    ) -> Result<()> {
        let pending = {
            let state = self.state.lock().expect("session state poisoned");
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

    /// Rename the conversation, so the list says what it is.
    ///
    /// ⚠ **A control request, not `/rename`**, and that is the whole of why this
    /// exists: a slash command written to a working session is parked and handed
    /// to the MODEL as words — measured, and the agent politely said "nothing for
    /// me to do" while the name never changed. See [`protocol::rename`].
    ///
    /// Nothing is recorded here on the way out. The CLI writes a `custom-title`
    /// line to the transcript, which is where the roster reads every name from
    /// ([`crate::past::about`]), so the new one arrives by the same route as a
    /// rename typed in a terminal — and a request that failed leaves the old name
    /// standing rather than a claim nobody checked.
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

    /// Change what this session may do without asking.
    ///
    /// ⚠ **Recorded optimistically.** The CLI answers with a `control_response`
    /// and this does not wait for it — see [`protocol::set_mode`] — so what the
    /// header shows is what was *asked for*, not a confirmation. That is the
    /// honest trade for not freezing a client behind a busy session, and it is
    /// why the mode is written only after stdin has taken the line: a write that
    /// failed leaves the old mode on screen, which is the true one.
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
        let mut state = self.state.lock().expect("session state poisoned");
        // Kept so a refusal can put back the mode the session is really in. Only
        // when nothing is already outstanding: two changes in flight and the
        // second would record the first's optimistic value as the truth to
        // return to, which is the same defect one step along.
        if state.restore.is_none() {
            state.restore = state.mode.clone();
        }
        // The old explanation goes with the old attempt.
        state.mode_refused = None;
        state.mode = Some(mode.to_string());
        Ok(())
    }

    /// Take the CLI at its word about what mode it is in.
    ///
    /// ⚠ **This is the correction [`Session::set_mode`]'s optimism depends on.**
    /// Without it a mode the CLI refused stayed on screen for the life of the
    /// session: a switch to `bypassPermissions` read *Bypass Permissions* in the
    /// header while the CLI stayed in `auto` and went on asking for approval
    /// (memview #96). Optimism between asking and hearing back is a fair trade
    /// for not freezing a client behind a busy session; a claim that is never
    /// corrected is not.
    ///
    /// **The confirmed mode comes from the reply**, not from what was asked for
    /// — see [`protocol::mode_reply`], where the measured shapes are written
    /// down. Taking "it succeeded" as agreement about *which* mode would be the
    /// same mistake at one remove.
    fn settle_mode(&self, reply: protocol::ModeReply) {
        let mut state = self.state.lock().expect("session state poisoned");
        match reply {
            protocol::ModeReply::Now(mode) => {
                state.mode = Some(mode);
                state.mode_refused = None;
            }
            protocol::ModeReply::Refused(why) => {
                tracing::info!("{}: the mode change was refused — {why}", self.id);
                // Back to what it was. `restore` is empty only if a reply
                // arrived for a change this console did not make, in which case
                // there is nothing it can honestly put back.
                if let Some(was) = state.restore.clone() {
                    state.mode = Some(was);
                }
                state.mode_refused = Some(why);
            }
        }
        state.restore = None;
    }

    /// Keep what the CLI answered about each window.
    ///
    /// Overwrites per window rather than wholesale, on the same reasoning as the
    /// stream events: an answer that names one window says nothing about
    /// another, and this reply and those events write to the same place.
    fn record_usage(&self, windows: Vec<(String, f64, Option<i64>)>) {
        let mut state = self.state.lock().expect("session state poisoned");
        let at = Heard(now());
        for (window, utilization, resets_at) in windows {
            state.spent.insert(
                window,
                Seen {
                    utilization,
                    resets_at: resets_at.map(ResetsAt),
                    at,
                },
            );
        }
    }

    /// Ask this session what the account has spent. See [`protocol::get_usage`].
    ///
    /// The answer does not come back here — it arrives on stdout like everything
    /// else and is recorded as it passes [`Self::apply`], which is what makes one
    /// question enough for every client watching. Failure is not worth
    /// propagating: a session that will not take the question is one whose
    /// figures the console simply does not have.
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
    ///
    /// Closing stdin is the exit the CLI is built for. The timer behind it is
    /// there because a session that will not end must not be able to keep the
    /// console holding a handle to it for ever.
    ///
    /// ⚠ **The deadline is recorded as well as slept on, because the sleep does
    /// not survive an upgrade.** `handover` re-execs this process, and a
    /// `tokio::spawn` is part of the image that goes; the session is not carried
    /// either, since closing stdin makes its descriptors unkeepable. Both at
    /// once left a stopped session running for two and a quarter hours
    /// (memview #750) — a child of the console with no row anywhere in it, which
    /// is precisely the state [`crate::roster::Roster::kill_all`] exists to
    /// avoid. So the *when* is written down where the handover can read it, and
    /// the new image finishes what this one started.
    pub async fn stop(self: &Arc<Self>) {
        self.stdin.lock().await.take();
        self.state.lock().expect("session state poisoned").stopping =
            Some(now() + GRACE.as_millis() as i64);
        let session = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(GRACE).await;
            session.force();
        });
    }

    /// When this session's kill falls due, for one that has been stopped.
    ///
    /// `None` for a session nobody has stopped, which is every session that is
    /// simply running.
    pub fn stopping(&self) -> Option<i64> {
        self.state.lock().expect("session state poisoned").stopping
    }

    /// Kill the session now.
    pub fn force(&self) {
        if let Some(kill) = self.kill.lock().expect("session kill poisoned").take() {
            // A spawned session: the reaper holds the child and kills it.
            if kill.send(()).is_ok() {
                return;
            }
        }
        // An adopted one has no child handle — the image that owned it exec'd
        // away — so the pid is the only handle left.
        if self.pid != 0 {
            // SAFETY: a kill to a pid this console started; the worst case is
            // ESRCH for a process that has already gone, which is ignored.
            unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL) };
        }
    }

    /// Record an event as having happened now.
    fn push(&self, event: Event) {
        self.push_at(event, Some(now()));
    }

    /// Note that the process said something, whatever it was.
    ///
    /// Not the same as the transcript's last activity, which is about the
    /// conversation and is read off the file. This is about the *process*: which
    /// of them is currently talking to the API, and therefore which one holds a
    /// current answer to `get_usage` rather than a cache from whenever it last
    /// made a request. See [`crate::roster::Roster::ask_usage`].
    fn heard(&self) {
        self.state.lock().expect("session state poisoned").heard = now();
    }

    /// When this session's process last said anything. See [`Self::heard`].
    pub fn last_heard(&self) -> i64 {
        self.state.lock().expect("session state poisoned").heard
    }

    /// How long this session has been failing to read what was written to it,
    /// in milliseconds — `None` for one that is merely busy, or quiet.
    ///
    /// ⚠ **The console has always held the evidence and never drawn the
    /// conclusion.** A message written to a session that has stopped reading its
    /// stdin gets an *Accepted*, which the client draws as *waiting to be read* —
    /// the identical words it uses for a message a working session will get to in
    /// a minute. On 2026-08-08 `hardware` went deaf twice in seventy-five
    /// minutes and both times the screen said the ordinary thing, so both times
    /// somebody had to work out by hand that it was not ordinary. See
    /// [`crate::past`] and the memory `reference_console_session_stops_reading_stdin`.
    ///
    /// **Three things at once, and the conjunction is the point:**
    ///
    /// * a message is in flight — nothing to read is not deafness;
    /// * the session is between turns ([`State::idle_since`]) — a session
    ///   working through a ten-minute tool call is silent and perfectly well,
    ///   and it parks input on purpose;
    /// * long enough — [`DEAF_AFTER_MS`], or [`DEAF_AFTER_COMPACT_MS`] while a
    ///   compaction is outstanding.
    ///
    /// The clock starts at whichever came second, the turn ending or the message
    /// arriving: before both of those the session has not yet been given the
    /// chance this measures.
    ///
    /// ⚠ **It cannot see a session that goes deaf mid-turn**, because there is
    /// nothing to distinguish that from work. Both measured episodes were between
    /// turns, which is also what the failure mode predicts — the reader stops
    /// when it goes back to waiting on the pipe.
    pub fn deaf(&self) -> Option<i64> {
        deaf_for(&self.state.lock().expect("session state poisoned"))
    }

    /// Say so, once, if this session has stopped reading.
    ///
    /// Returns how long it has been deaf when this is the call that noticed —
    /// `None` on every later sweep of the same episode, and `None` for a session
    /// that is fine. Swept rather than pushed from the read loop because
    /// deafness is the absence of events, and nothing arrives to trigger it.
    ///
    /// The pid comes back with it because the caller's next job is to capture
    /// what the process looks like before the cure destroys it — see
    /// [`crate::roster::Roster::watch_for_deafness`].
    pub fn check_deaf(&self) -> Option<(u64, usize)> {
        let mut state = self.state.lock().expect("session state poisoned");
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

    /// What this session was last told it may do without asking. See
    /// [`Summary::mode`] — the console is the only thing that knows.
    pub fn mode(&self) -> Option<String> {
        self.state
            .lock()
            .expect("session state poisoned")
            .mode
            .clone()
    }

    /// What was written to this session and never read, oldest first.
    ///
    /// The other half of the cure: a restart loses whatever is still sitting in
    /// the old pipe, so it has to be given back afterwards. See
    /// [`crate::roster::Roster::revive`].
    pub fn unread(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("session state poisoned")
            .unread
            .iter()
            .map(|held| held.text.clone())
            .collect()
    }

    /// Record an event and hand it to whoever is listening.
    ///
    /// `at` is passed rather than taken because a seeded event did not happen
    /// now — it happened whenever the transcript says, which for a resumed
    /// conversation may be weeks ago. Stamping those with the clock would put
    /// today's date on every line of a conversation from June.
    fn push_at(&self, event: Event, at: Option<i64>) {
        let stamped = {
            let mut state = self.state.lock().expect("session state poisoned");
            match &event {
                Event::Started { model, .. } => state.model = Some(model.clone()),
                Event::Busy { status } => state.busy = Some(status.clone()),
                // The window, which only the result line declares. How full it
                // is arrives per message — see [`Event::Context`].
                Event::Context { tokens } => state.context = Some(*tokens),
                // Everything the last measurement counted was replaced by a
                // summary, so it is not a stale number — it is another
                // conversation's. Cleared rather than estimated: a plausible
                // figure is indistinguishable on screen from a measured one, and
                // the client already draws nothing where there is nothing. The
                // next message brings a real one.
                //
                // Seen only in a replayed transcript; the live path is
                // [`Self::recount`], which reads the file for the same reason.
                Event::Compacted => state.context = None,
                Event::Turn {
                    cost_usd, window, ..
                } => {
                    if window.is_some() {
                        state.window = *window;
                    }
                    state.busy = None;
                    // ⚠ **Assigned, not added.** The field behind this is the
                    // CLI's `total_cost_usd`, and it means what it says: the
                    // running total for the session so far, not the price of
                    // the exchange that just ended. Adding those totals to each
                    // other yields a triangular sum — measured on a live
                    // session reading 3.03, 3.76, 8.00, 9.55, 10.66, 11.97,
                    // 12.35, where `+=` had reached $59.32 against a true
                    // $12.35.
                    state.cost_usd = *cost_usd;
                    // The turn's own `num_turns` is deliberately not read: it
                    // counts the assistant messages the exchange took, which is
                    // a different question from how many exchanges there have
                    // been. [`Self::recount`] answers that one, from the file.
                }
                Event::Limit {
                    window,
                    status,
                    resets_at,
                    utilization,
                } => {
                    state.limit = Some(status.clone());
                    // ⚠ **One window per event**, so they are collected as they
                    // are seen rather than replaced wholesale: an event about
                    // the five-hour window says nothing about the weekly one,
                    // and overwriting would lose whichever was not mentioned.
                    if let Some(spent) = utilization {
                        state.spent.insert(
                            window.clone(),
                            Seen {
                                utilization: *spent,
                                resets_at: resets_at.map(ResetsAt),
                                at: Heard(now()),
                            },
                        );
                    }
                }
                Event::Prompt { text } if state.asked.is_none() => {
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
                    // Nothing can be approved for a process that has gone, and a
                    // question left standing would keep saying the session is
                    // waiting for someone.
                    state.pending.clear();
                    // Nor can a command be written to it. Held ones were waiting
                    // for a turn to end that now never will, and a chip promising
                    // one is about to run is the same lie this whole mechanism
                    // exists to stop.
                    state.held.clear();
                }
                _ => {}
            }
            // Remember what each call IS, so that if it turns out to have
            // detached, the strip can name it. See `State::called`.
            if let Event::Tool { id, name, input } = &event {
                if state.called.len() >= CALLED_RING {
                    state.called.pop_front();
                }
                state
                    .called
                    .push_back((id.clone(), crate::protocol::called(name, input)));
            }
            // Work left running, which is a different question about the same
            // event and is decided where the events are read rather than here.
            // See [`protocol::running`] for the two cases that mean "forget what
            // you were counting".
            match protocol::running(&event) {
                protocol::Running::Began { tool, task } => {
                    // Unnamed rather than absent when the ring has already
                    // rolled past it: that it is running is the fact worth
                    // keeping, and a call with no name still beats a bare count.
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
                // By the task, because that is the only name a kill gives — and
                // the call it belonged to is never heard from again.
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
        // An error here means nobody is listening, which is the normal state of
        // a session working on its own.
        let _ = self.tx.send(stamped);
    }

    fn note_stderr(&self, line: &str) {
        let mut state = self.state.lock().expect("session state poisoned");
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
            .expect("session state poisoned")
            .log
            .iter()
            .map(|stamped| stamped.event.clone())
            .collect()
    }

    /// What a client that says it holds everything through `after` still needs.
    ///
    /// Resuming is refused rather than approximated — see [resumable] for when,
    /// and why a refusal is the kinder answer.
    pub fn since(&self, after: Option<u64>) -> Backlog {
        let state = self.state.lock().expect("session state poisoned");
        // With an empty log nothing is held, so the earliest number that could
        // still be honoured is the next one to be issued.
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

    pub fn listen(&self) -> broadcast::Receiver<Stamped> {
        self.tx.subscribe()
    }

    /// What the child said on stderr, for when it will not start.
    pub fn trouble(&self) -> String {
        self.state
            .lock()
            .expect("session state poisoned")
            .stderr
            .clone()
    }

    pub fn alive(&self) -> bool {
        self.state.lock().expect("session state poisoned").alive
    }

    /// Whether a turn is running right now. See [`Summary::working`], and ⚠ not
    /// [`Summary::busy`], which cannot answer this — a status is announced when
    /// it *changes*, so a long stretch of one activity leaves nothing standing.
    pub fn working(&self) -> bool {
        self.state.lock().expect("session state poisoned").working
    }

    pub fn summary(&self) -> Summary {
        let state = self.state.lock().expect("session state poisoned");
        Summary {
            id: self.id.clone(),
            dir: self.dir.display().to_string(),
            started: self
                .started
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            // Left for the roster, which reads the transcript once per listing
            // and already goes there for the name.
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
            waiting: state.pending.len(),
            unread: state.unread.len(),
            deaf: deaf_for(&state).map(|ms| (ms / 1000) as u64),
            held: state.held.iter().cloned().collect(),
        }
    }
}

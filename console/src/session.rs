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
/// Two numbers are deliberately absent because the file holds them better: how
/// full the context is, which is on every assistant message, and how many
/// exchanges there have been ([`crate::past::interactions`]). Anything derivable
/// from the transcript is derived, because that survives a cold start too — and
/// this only survives an upgrade.
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

/// How much transcript one session keeps in memory.
///
/// The console holds no database in phase 1 and the transcripts on disk are the
/// durable record, so this is a scrollback, not an archive.
const SCROLLBACK: usize = 5000;

/// How long a session gets to finish after its stdin closes, before it is killed.
///
/// Generous on purpose: the process may be mid-tool-call, and the clean exit is
/// worth waiting for because it is the one that flushes the transcript.
const GRACE: Duration = Duration::from_secs(30);

/// How much of the child's stderr to keep for diagnosis.
const STDERR_KEPT: usize = 4000;

/// What a client sees of a session without reading its transcript.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub id: String,
    pub dir: String,
    /// Seconds since the epoch.
    pub started: u64,
    pub alive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// What the CLI last said it was doing, when it is doing anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy: Option<String>,
    /// How many times someone has spoken to this session since it was last
    /// compacted — exchanges, not messages, and not the result line's
    /// `num_turns`. See [`crate::past::interactions`] for why it is counted from
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
    /// Filled in by the roster from the transcript, for the same reason as
    /// [`Self::name`]: the mode changes while the session runs and the stream
    /// announces it only once, on the init line. **Stored names are not the
    /// displayed ones** — `default` is shown as *Manual* — so the client keeps
    /// the CLI's own table rather than prettifying these itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// How many questions it is blocked on. The one number that means "this
    /// session cannot go on without you", so it belongs in the list of sessions
    /// and not only on the page of one.
    pub waiting: usize,
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

/// Now, in milliseconds since the epoch.
fn now() -> i64 {
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

/// The mutable half of a session, behind one lock.
#[derive(Debug, Default)]
struct State {
    log: VecDeque<Stamped>,
    /// The last sequence number issued. Not the log's length: the log is a
    /// scrollback and drops its front, and a number that was reused after an
    /// eviction would resume a client into the wrong place.
    issued: u64,
    /// Questions the session is blocked on, by control-request id, with the
    /// arguments it asked about — an allow has to echo them back.
    pending: BTreeMap<String, serde_json::Value>,
    alive: bool,
    model: Option<String>,
    busy: Option<String>,
    /// See [`Summary::interactions`]. Derived from the transcript, never
    /// accumulated, so nothing here needs carrying across an upgrade.
    interactions: u32,
    cost_usd: f64,
    /// The last turn's prompt size and the window it went into. See
    /// [`Summary::context`].
    context: Option<u64>,
    window: Option<u64>,
    /// See [`Summary::limit`].
    limit: Option<String>,
    asked: Option<String>,
    stderr: String,
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

    /// Recount the exchanges from the transcript. See [`crate::past::interactions`].
    ///
    /// Silent when there is no transcript yet — a session that has just been
    /// started has none, and reporting zero for it is right anyway. Reads the
    /// file before taking the lock, which is the whole reason this is a method
    /// and not a line in `push_at`.
    fn recount(&self) {
        let root = crate::past::projects_root();
        let Some(path) = crate::past::transcript_of(&root, &self.id) else {
            return;
        };
        let counted = crate::past::interactions(&path);
        self.state
            .lock()
            .expect("session state poisoned")
            .interactions = counted;
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
    pub fn adopt(id: String, dir: PathBuf, pid: u32, fds: Fds, tally: Tally) -> Result<Arc<Self>> {
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
                cost_usd: tally.cost_usd,
                window: tally.window,
                limit: tally.limit,
                ..State::default()
            }),
            stdin: tokio::sync::Mutex::new(Some(Box::new(stdin) as Sink)),
            pid,
            fds,
            kill: Mutex::new(Some(kill_tx)),
            tx,
        });
        session.seed();
        session.clone().read_from(Some(stdout), Some(stderr), true);
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
                while let Ok(Some(line)) = lines.next_line().await {
                    for event in protocol::read(&line) {
                        // The end of a turn is the one moment the exchange count
                        // can have changed, and by then the CLI has written the
                        // whole exchange to its transcript. Recounted rather than
                        // incremented — see [`crate::past::interactions`] — and
                        // done here rather than in `push_at`, which holds the
                        // state lock and must not be reading files.
                        let counted = matches!(event, Event::Turn { .. });
                        session.push(event);
                        if counted {
                            session.recount();
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
    pub async fn send(&self, text: &str) -> Result<()> {
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
        // Held even if the CLI never echoes it, so the record of what was asked
        // does not depend on the CLI's replay behaviour.
        let mut state = self.state.lock().expect("session state poisoned");
        if state.asked.is_none() {
            state.asked = Some(text.to_string());
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
    pub async fn decide(&self, id: &str, allowed: bool, why: &str) -> Result<()> {
        let input = {
            let state = self.state.lock().expect("session state poisoned");
            state
                .pending
                .get(id)
                .cloned()
                .context("that question is not open — it may already have been answered")?
        };
        let line = protocol::decision(id, allowed, &input, why);
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
        });
        Ok(())
    }

    /// End the session: close stdin, and kill it if it has not gone on its own.
    ///
    /// Closing stdin is the exit the CLI is built for. The timer behind it is
    /// there because a session that will not end must not be able to keep the
    /// console holding a handle to it for ever.
    pub async fn stop(self: &Arc<Self>) {
        self.stdin.lock().await.take();
        let session = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(GRACE).await;
            session.force();
        });
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
                Event::Limit { status, .. } => state.limit = Some(status.clone()),
                Event::Prompt { text } if state.asked.is_none() => {
                    state.asked = Some(text.clone());
                }
                Event::Ask { id, input, .. } => {
                    state.pending.insert(id.clone(), input.clone());
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
                }
                _ => {}
            }
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
            alive: state.alive,
            model: state.model.clone(),
            busy: state.busy.clone(),
            interactions: state.interactions,
            cost_usd: state.cost_usd,
            limit: state.limit.clone(),
            context: state.context,
            window: state.window,
            asked: state.asked.clone(),
            // Both filled in by the roster, which knows where the transcripts are.
            name: None,
            mode: None,
            waiting: state.pending.len(),
        }
    }
}

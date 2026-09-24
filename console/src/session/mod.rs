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
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, oneshot};

use crate::protocol::{self, Event};

mod input;
mod process;
mod state;
mod summary;
mod view;

pub use process::{Fds, finish, keepable, names_session};
pub use state::{Backlog, Pending, Stamped, deaf_after, resumable, working_after};
pub use summary::{Heard, ResetsAt, Seen, Summary, Tally};

use process::reap_adopted;
use state::{State, deaf_for, forget};

/// Where a session's instructions go. A trait object because a session is
/// spawned, with the child's own [`ChildStdin`], or adopted across an upgrade,
/// with the same pipe reopened from a raw descriptor.
type Sink = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// How long a session gets to finish after its stdin closes, before it is killed.
/// Generous: the clean exit is the one that flushes the transcript.
const GRACE: Duration = Duration::from_secs(30);

/// How much of the child's stderr to keep for diagnosis.
const STDERR_KEPT: usize = 4000;

/// How often to re-read the transcript while the child says nothing. The read
/// is incremental, so this is set by how long a wrong number may stay on screen.
const RECOUNT_EVERY: Duration = Duration::from_secs(5);

/// Now, in milliseconds since the epoch.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
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
    /// What this session is called to its peers — `-n`, which is the only thing that
    /// writes a chosen name into `~/.claude/sessions/<pid>.json`, the file
    /// `ListAgents` reads and `SendMessage` addresses. Without it every session in
    /// one directory derives the same name from that directory and is told apart
    /// only by a hash, so a session asked to reach `memview` finds no such peer.
    /// `rename_session` does not do this: the CLI calls it the user-facing title and
    /// it goes no further than the transcript. See [`crate::roster::Roster::resume`],
    /// which is where a conversation's own name becomes this argument.
    pub name: Option<String>,
    /// What the session may do without being asked. In headless mode there is nobody
    /// to answer a prompt, so under the CLI's default every tool call needing
    /// permission is refused.
    pub permission_mode: Option<crate::modes::Mode>,
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
        // would say `just now` about a conversation opened after two days. See
        // [`crate::past::last_moved`].
        let session = Self::spawn(id, dir, spawn, true)?;
        // A new `claude` on an old conversation: whatever the transcript shows still
        // running was written by a process that is gone.
        session.seed(true);
        Ok(session)
    }

    /// Put what was already said in front of what happens next. `--resume` restores
    /// the CLI's context and replays none of it on stdout, so without this a resumed
    /// session opens empty. The same vocabulary the stream uses, read the other way
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
        // the last page and full of prompts. Set unconditionally, including to `None`;
        // `origin_read` is what makes `None` stick. See [`crate::past::opening`].
        {
            let mut state = self.state.lock();
            state.asked = crate::past::opening(&path);
            state.origin_read = true;
        }
        self.recount();
    }

    /// Count the exchanges the transcript has gained — [`crate::past::counted`].
    /// Reads the file outside the lock, which is why this is a method and not a line
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
            command.args(["--permission-mode", mode.name()]);
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
                        .unwrap_or(crate::modes::Mode::Default),
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
        // The scrollback does not survive the exec, so an adopted session reseeds from
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
        // The same child across the exec, so nothing above the boundary is dead on
        // account of the upgrade — see [`crate::protocol::Event::Joined`].
        session.seed(false);
        // After the seed, so the question lands at the end of the conversation, where
        // it is still standing. Pushed as an ordinary `Ask`: one mechanism, so a
        // reconnecting client is offered the decision again and the session is
        // recorded as waiting for it.
        for (id, question) in pending {
            session.push(question.ask(id));
        }
        // After the seed, because the seed ends with a `Joined`, which
        // `protocol::running` reads as `Running::Gone` — right for a resume, wrong for
        // an adoption whose children are still running.
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

    /// Read the child's streams until they end. `ends_on_eof` decides who declares
    /// the session over: for an adopted session end of file is the only signal; for
    /// a spawned one [`Self::reap`] must say so, because only it knows the exit code,
    /// and letting the reader win turns every clean exit into `code: None`.
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
                        // to the file and announced on no stream, so a read tied to `Turn` alone leaves
                        // a stale fullness on screen.
                        _ = beat.tick() => {
                            session.recount();
                            continue;
                        }
                    };
                    let Ok(Some(line)) = line else { break };
                    // Every line, whatever it is: the record of when the process last spoke, which
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
                        // which holds the state lock and must not read files; the count is wanted now.
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

    /// Note that the process said something, whatever it was. About the process,
    /// not the conversation: which one holds a current answer to `get_usage`. See
    /// [`crate::roster::Roster::ask_usage`].
    fn heard(&self) {
        self.state.lock().heard = now();
    }

    /// Say so, once, if this session has stopped reading: how long, when this is the
    /// call that noticed, and `None` on later sweeps of the same episode. The pid
    /// comes back with it so the caller can capture the process before the cure
    /// destroys it — [`crate::roster::Roster::watch_for_deafness`].
    pub fn check_deaf(&self) -> Option<(u64, usize)> {
        let mut state = self.state.lock();
        let seconds = (deaf_for(&state, now())? / 1000) as u64;
        if state.announced_deaf {
            return None;
        }
        state.announced_deaf = true;
        let unread = state.unread.len();
        drop(state);
        self.push(Event::Deaf { unread, seconds });
        Some((seconds, unread))
    }

    /// Record an event and hand it to whoever is listening. `at` is passed rather
    /// than taken because a seeded event happened whenever the transcript says.
    fn push_at(&self, event: Event, at: Option<i64>) {
        let stamped = self.state.lock().take(event, at, now());
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
}

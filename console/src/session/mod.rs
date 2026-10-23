//! One live Claude Code session: the subprocess, its transcript, its listeners.
//!
//! One process serves turn after turn on an open stdin and exits 0 when it closes.
//! The id is ours, chosen before the process exists, so a client can subscribe to
//! a session still starting.

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

pub use process::{Fds, children_started, finish, keepable, names_session};
pub use state::{
    Backlog, Pending, Stamped, deaf_after, leaves_a_decision, resumable, taken_up, working_after,
};
pub use summary::{Heard, ResetsAt, Seen, Summary, Tally};

use process::reap_adopted;
use state::{State, deaf_for, forget};

/// Where a session's instructions go: the child's stdin, or the same pipe reopened
/// from a raw descriptor after an upgrade.
type Sink = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// How long a session gets to exit after its stdin closes. Generous: a clean exit
/// flushes the transcript.
const GRACE: Duration = Duration::from_secs(30);

/// How much of the child's stderr to keep for diagnosis.
const STDERR_KEPT: usize = 4000;

/// How often to re-read the transcript while the child is quiet: how long a wrong
/// number may stay on screen.
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

/// By hand: the sink is a trait object.
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
    /// What peers call this session: `-n`, the only way a name reaches
    /// `~/.claude/sessions/<pid>.json`, which `SendMessage` addresses.
    /// `rename_session` reaches only the transcript.
    pub name: Option<String>,
    /// What the session may do without asking.
    pub permission_mode: Option<crate::modes::Mode>,
}

impl Session {
    /// Start a session in `dir`, with `id` as both our handle and its session id.
    pub fn start(id: String, dir: &Path, spawn: &Spawn) -> Result<Arc<Self>> {
        Self::spawn(id, dir, spawn, false)
    }

    /// Pick up an existing conversation by `--resume`, which refuses an unknown id
    /// rather than starting a fresh session under it.
    pub fn resume(id: String, dir: &Path, spawn: &Spawn) -> Result<Arc<Self>> {
        let session = Self::spawn(id, dir, spawn, true)?;
        // Whatever the transcript shows running belonged to a process that is gone.
        session.seed(true);
        Ok(session)
    }

    /// Put what was already said in front of what happens next: `--resume` replays
    /// none of it on stdout. Read by [`crate::protocol::read_recorded`].
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
        // Between what was read and what is watched, carrying the cursor.
        self.push(Event::Joined {
            earlier: count,
            from: seed.from,
            restarted,
        });
        // From the head of the file, not the replayed page; `origin_read` makes a
        // `None` stick.
        {
            let mut state = self.state.lock();
            state.asked = crate::past::opening(&path);
            state.origin_read = true;
        }
        self.recount();
    }

    /// Count the exchanges the transcript has gained — [`crate::past::counted`].
    /// Reads the file outside the lock; call it from the reading task only.
    fn recount(&self) {
        let root = crate::past::projects_root();
        let Some(path) = crate::past::transcript_of(&root, &self.id) else {
            return;
        };
        let mut so_far = self.state.lock().counted;
        // Zero would read the whole file, gigabytes, on the executor.
        if so_far.through == 0 {
            so_far.through = crate::past::seed_from(&path);
        }
        let found = crate::past::counted(&path, so_far);
        let mut state = self.state.lock();
        state.counted = found.counted;
        // Work the file says finished, which no event announces.
        for named in &found.finished {
            forget(&mut state.background, named);
        }
        // A compaction is announced only in the file.
        if found.compacted {
            state.context = found.context;
        }
        // So is a switch of model or effort; `init` named only the first model.
        if let Some(answered) = found.answered {
            // An answer says `claude-opus-5` where `init` said `claude-opus-5[1m]`: the
            // variant is the init's to say, a million tokens of window.
            let variant = state
                .model
                .as_deref()
                .is_some_and(|model| model.split('[').next() == Some(answered.model.as_str()));
            if !variant {
                state.model = Some(answered.model);
            }
            state.effort = answered.effort;
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
            // Unlisted in `--help`; without it no permission question reaches us.
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
        // Before the handles move: after an upgrade these numbers are the connection.
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
                // Unset is the CLI's default mode, not unknown.
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

    /// Take over a session the previous image ran: same pid and pipes, no [`Child`]
    /// handle, so exit is noticed at end of file and a kill goes by pid. The
    /// scrollback is reseeded from the transcript; [`Tally`] is carried.
    pub fn adopt(
        id: String,
        dir: PathBuf,
        pid: u32,
        fds: Fds,
        mut tally: Tally,
    ) -> Result<Arc<Self>> {
        let pending = std::mem::take(&mut tally.pending);
        // Put back after the seed, like `pending`.
        let background = std::mem::take(&mut tally.background);
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
                effort: tally.effort,
                mode: tally.mode,
                // Before the seed, which sets `asked` only when it is `None`.
                asked: tally.asked,
                cost_usd: tally.cost_usd,
                window: tally.window,
                limit: tally.limit,
                // Nothing on disk records it.
                spent: tally.spent,
                counted: tally.counted,
                // Still in flight; its next status or `Turn` corrects it.
                busy: tally.busy,
                ..State::default()
            }),
            stdin: tokio::sync::Mutex::new(Some(Box::new(stdin) as Sink)),
            pid,
            fds,
            kill: Mutex::new(Some(kill_tx)),
            tx,
        });
        // The same child across the exec: nothing above the boundary died.
        session.seed(false);
        // After the seed, at the end where it still stands, as an ordinary `Ask`.
        for (id, question) in pending {
            session.push(question.ask(id));
        }
        // After the seed, whose `Joined` clears the count — wrong for an adoption.
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

    /// Read the child's streams until they end. `ends_on_eof` for an adopted session,
    /// which has no other signal; a spawned one is ended by [`Self::reap`], which knows
    /// the exit code.
    fn read_from<O, E>(self: Arc<Self>, stdout: Option<O>, stderr: Option<E>, ends_on_eof: bool)
    where
        O: tokio::io::AsyncRead + Unpin + Send + 'static,
        E: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        if let Some(stdout) = stdout {
            let session = self.clone();
            tokio::spawn(async move {
                // A new process and an adopted one alike: asking twice is harmless.
                session.ask_commands().await;
                let mut lines = BufReader::new(stdout).lines();
                let mut beat = tokio::time::interval(RECOUNT_EVERY);
                // Delay, not Burst: a session busy for a minute owes one catch-up read.
                beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    let line = tokio::select! {
                        line = lines.next_line() => line,
                        // The file changes while the process is quiet, e.g. a compaction.
                        _ = beat.tick() => {
                            session.recount();
                            continue;
                        }
                    };
                    let Ok(Some(line)) = line else { break };
                    // Every line: who to ask for usage — [`Session::heard`].
                    session.heard();
                    // Answers to the console are not part of the conversation; the
                    // mode's is settled against what was asked — [`Session::settle_mode`].
                    let events = match protocol::output(&line) {
                        protocol::Output::Commands(commands) => {
                            session.record_commands(commands);
                            continue;
                        }
                        protocol::Output::Usage(reply) => {
                            session.record_usage(reply);
                            continue;
                        }
                        protocol::Output::Mode(reply) => {
                            session.settle_mode(reply);
                            continue;
                        }
                        protocol::Output::Events(events) => events,
                    };
                    for event in events {
                        // A turn's end is when the exchange count changes; counted here,
                        // outside the lock `push_at` holds.
                        let counted = matches!(event, Event::Turn { .. });
                        session.push(event);
                        if counted {
                            session.recount();
                            // Commands parked mid-turn go now. Logged, not propagated: ending
                            // this loop would end the transcript.
                            if let Err(err) = session.release_held().await {
                                tracing::warn!("{}: holding a command back: {err:#}", session.id);
                            }
                        }
                    }
                }
                // The pipe closed; say so only when no reaper will.
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

    /// Record that the process is gone, once: reader and reaper can both notice.
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

    /// End the session: close stdin, and kill it if it lingers. The deadline is also
    /// recorded, for [`crate::roster::Roster::finish_stopping`] after an upgrade.
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

    /// A command waiting for a root password. See [`crate::sudo`].
    pub fn ask_sudo(&self, id: &str, command: &str) {
        let input = serde_json::json!({ "command": command });
        self.push(Event::Ask {
            id: id.to_string(),
            call: None,
            does: crate::call::Call::read("Bash", &input),
            tool: crate::sudo::TOOL.to_string(),
            title: None,
            detail: None,
            input,
        });
    }

    /// The root password question settled: a password given, or refused.
    pub fn sudo_settled(&self, id: &str, given: bool) {
        self.push(Event::Answered {
            id: id.to_string(),
            allowed: given,
            reply: None,
        });
    }

    /// Record an event as having happened now.
    fn push(&self, event: Event) {
        self.push_at(event, Some(now()));
    }

    /// The process said something: it holds a current answer to `get_usage`. See
    /// [`crate::roster::Roster::ask_usage`].
    fn heard(&self) {
        self.state.lock().heard = now();
    }

    /// End background work more than `grace` ms past its declared deadline, and log
    /// it: its closing notice never came, or came in new words.
    pub fn end_overdue(&self, grace: i64) -> Vec<crate::protocol::Called> {
        let ended = self.state.lock().overdue(now(), grace);
        for called in &ended {
            tracing::warn!(
                "{}: {} {} passed its declared deadline with no ending this console recognised — \
                 taken off the count; look for a closing notice in new words",
                self.id,
                called.tool,
                called.label.as_deref().unwrap_or("(unlabelled)"),
            );
        }
        ended
    }

    /// Clear the decision clock if the call it allowed is running — [`taken_up`] —
    /// and the clock still holds the decision judged.
    pub fn take_up_decision(&self) {
        let Some(decided) = self.state.lock().decided else {
            return;
        };
        let Some(children) = children_started(self.pid) else {
            return;
        };
        if taken_up(decided, &children) {
            let mut state = self.state.lock();
            if state.decided == Some(decided) {
                state.decided = None;
            }
        }
    }

    /// Say so, once per episode, if this session has stopped reading: how long, and
    /// how many messages wait. `None` on later sweeps of the same episode.
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

    /// Record an event and hand it to listeners. `at`, because a seeded event
    /// happened when the transcript says.
    fn push_at(&self, event: Event, at: Option<i64>) {
        let stamped = self.state.lock().take(event, at, now());
        // No listener is normal.
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

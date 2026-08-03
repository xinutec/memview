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
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, oneshot};

use crate::protocol::{self, Event};

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
    pub turns: u32,
    pub cost_usd: f64,
    /// The first thing this session was asked to do, kept as its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asked: Option<String>,
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
    pub event: Event,
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
    turns: u32,
    cost_usd: f64,
    asked: Option<String>,
    stderr: String,
}

#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub dir: PathBuf,
    started: SystemTime,
    state: Mutex<State>,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    tx: broadcast::Sender<Stamped>,
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
        let earlier = crate::past::replay(&path);
        tracing::info!(
            "seeded {} with {} events from its transcript",
            self.id,
            earlier.len()
        );
        let count = earlier.len();
        for event in earlier {
            self.push(event);
        }
        // Last, so it sits between what was read and what we watch.
        self.push(Event::Joined { earlier: count });
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
            stdin: tokio::sync::Mutex::new(Some(stdin)),
            kill: Mutex::new(Some(kill_tx)),
            tx,
        });

        session.clone().watch(child, kill_rx);
        Ok(session)
    }

    /// Read the child's streams until it ends, then record how it ended.
    fn watch(self: Arc<Self>, mut child: Child, kill: oneshot::Receiver<()>) {
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        if let Some(stdout) = stdout {
            let session = self.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    for event in protocol::read(&line) {
                        session.push(event);
                    }
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

        tokio::spawn(async move {
            let code = tokio::select! {
                status = child.wait() => status.ok().and_then(|s| s.code()),
                _ = kill => {
                    let _ = child.kill().await;
                    None
                }
            };
            self.push(Event::Exited { code });
            self.state.lock().expect("session state poisoned").alive = false;
        });
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
            let _ = kill.send(());
        }
    }

    /// Record an event and hand it to whoever is listening.
    fn push(&self, event: Event) {
        let stamped = {
            let mut state = self.state.lock().expect("session state poisoned");
            match &event {
                Event::Started { model, .. } => state.model = Some(model.clone()),
                Event::Busy { status } => state.busy = Some(status.clone()),
                Event::Turn {
                    cost_usd, turns, ..
                } => {
                    state.busy = None;
                    state.cost_usd += cost_usd;
                    state.turns += turns;
                }
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
            turns: state.turns,
            cost_usd: state.cost_usd,
            asked: state.asked.clone(),
            waiting: state.pending.len(),
        }
    }
}

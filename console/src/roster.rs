//! Every session the console owns. Sessions stay listed after they end: one that
//! failed to start is the thing most worth seeing.

#![expect(
    unsafe_code,
    reason = "env::remove_var, unsafe under edition 2024, on a path with no other thread"
)]

use anyhow::Context as _;
use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use anyhow::Result;

use crate::config::Config;
use crate::protocol::Event;
use crate::session::{Session, Stamped, Summary};
use tokio::sync::broadcast;

/// How long [`Roster::revive`] waits for a stopped session to leave the process table.
const REVIVE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(90);

pub struct Roster {
    config: Config,
    sessions: RwLock<BTreeMap<String, Arc<Session>>>,
    /// The account's rate-limit figure — see [`crate::usage`].
    usage: Arc<crate::usage::Usage>,
    /// What each conversation is about, in a sentence. See [`crate::gist`].
    gists: Arc<crate::gist::Gists>,
    drafts: Arc<crate::drafts::Drafts>,
    /// Each session's task list, between sweeps. See [`crate::tasks::Tasks`].
    tasks: Arc<crate::tasks::Tasks>,
    /// What each conversation was last allowed to do without asking — [`crate::modes`].
    modes: Arc<crate::modes::Modes>,
    /// Each transcript's landmarks, walked once then extended — [`crate::marks`].
    marks: Arc<crate::marks::Marks>,
    /// What each `Bash` call changed in the files it wrote. See [`crate::edits`].
    edits: Arc<crate::edits::Edits>,
    /// The truest reading of each rate-limit window, outliving the session that heard
    /// it — [`crate::usage::fresher`].
    spent: Mutex<BTreeMap<String, crate::session::Seen>>,
    /// Commands waiting for a root password. See [`crate::sudo`].
    sudo: crate::sudo::Waiting,
    login: crate::login::Login,
}

/// The environment variable an upgrade hands its sessions over in.
const HANDOVER: &str = "CONSOLE_HANDOVER";

/// And the one for the sessions it was stopping.
const STOPPING: &str = "CONSOLE_HANDOVER_STOPPING";

/// One session, as it travels across an upgrade.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Carried {
    id: String,
    dir: String,
    pid: u32,
    fds: crate::session::Fds,
    /// The counters no disk holds — see [`crate::session::Tally`].
    #[serde(default)]
    tally: crate::session::Tally,
}

/// A stopped session whose kill is still owed, carried apart: its stdin is closed.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Stopping {
    id: String,
    pid: u32,
    /// When the kill falls due, in epoch milliseconds — the original deadline.
    due: i64,
}

impl Roster {
    pub fn new(config: Config) -> Self {
        let usage = Arc::new(crate::usage::Usage::new(config.usage_url.clone()));
        let gists = Arc::new(crate::gist::Gists::load(config.gists.clone()));
        let drafts = Arc::new(crate::drafts::Drafts::load(config.drafts.clone()));
        let modes = Arc::new(crate::modes::Modes::load(config.modes.clone()));
        Self {
            config,
            sessions: RwLock::new(BTreeMap::new()),
            usage,
            gists,
            drafts,
            tasks: Arc::default(),
            modes,
            marks: Arc::default(),
            edits: Arc::new(crate::edits::Edits::new(crate::edits::edits_root())),
            spent: Mutex::new(BTreeMap::new()),
            sudo: crate::sudo::Waiting::default(),
            login: crate::login::Login::default(),
        }
    }

    /// What `Bash` calls changed in the files they wrote. See [`crate::edits`].
    pub fn sudo(&self) -> &crate::sudo::Waiting {
        &self.sudo
    }

    /// The sign-in under way from the phone, if any. See [`crate::login`].
    pub fn login(&self) -> &crate::login::Login {
        &self.login
    }

    /// The CLI every session runs, which is also the one that signs in.
    pub fn binary(&self) -> &str {
        &self.config.spawn.binary
    }

    pub fn edits(&self) -> Arc<crate::edits::Edits> {
        Arc::clone(&self.edits)
    }

    /// Every landmark in a conversation, walking only what is new. See [`crate::marks`].
    pub fn marks(&self) -> Arc<crate::marks::Marks> {
        Arc::clone(&self.marks)
    }

    /// Remember a conversation's mode for a later resume. See [`crate::modes`].
    pub fn remember_mode(&self, id: &str, mode: &crate::modes::Mode) {
        self.modes.set(id, mode);
    }

    /// Who holds which tasks, cached briefly — see [`crate::tasks::Tasks`].
    pub async fn tasks(&self) -> crate::tasks::Sweep {
        self.tasks.sweep().await
    }

    /// One conversation's tasks, for the sheet that lists them.
    pub async fn task_list(&self, session: &str) -> Vec<crate::tasks::Task> {
        self.tasks.listed(session).await
    }

    /// One task's prose, fetched when a row is opened.
    pub async fn task_detail(&self, task: &str) -> Option<String> {
        self.tasks.detail(task).await
    }

    /// The sentences, for the front page.
    pub fn gists(&self) -> BTreeMap<String, crate::gist::Gist> {
        self.gists.all()
    }

    /// Each conversation's unsent words.
    pub fn drafts(&self) -> Arc<crate::drafts::Drafts> {
        Arc::clone(&self.drafts)
    }

    /// Write a sentence for each conversation that has moved — [`crate::gist::Gists::sweep`].
    pub async fn write_gists(&self) {
        self.gists
            .sweep(&self.config.spawn.binary, &crate::past::projects_root())
            .await;
    }

    /// Drop pictures whose conversations are gone, off the runtime — [`crate::images::tidy`].
    pub async fn tidy_images(&self) {
        let done = tokio::task::spawn_blocking(|| {
            let keep = crate::past::transcript_ids(&crate::past::projects_root());
            crate::images::tidy(&crate::images::images_root(), &keep)
        })
        .await;
        match done {
            Ok(0) => {}
            Ok(gone) => tracing::info!("images: {gone} conversation(s) tidied away"),
            Err(why) => tracing::warn!("images: the tidy did not finish ({why})"),
        }
    }

    /// The rate-limit reading.
    pub fn usage(&self) -> &Arc<crate::usage::Usage> {
        &self.usage
    }

    /// Pick up the sessions an upgrade handed over — [`Self::handover`]. One that
    /// cannot be rebuilt is dropped, not guessed at: a wrong descriptor would send
    /// words to another session.
    pub fn inherit(&self) -> usize {
        let Ok(handed) = std::env::var(HANDOVER) else {
            return 0;
        };
        // So a later session does not think it was inherited.
        unsafe { std::env::remove_var(HANDOVER) };
        let carried: Vec<Carried> = match serde_json::from_str(&handed) {
            Ok(carried) => carried,
            Err(error) => {
                tracing::error!(
                    "the handover could not be read, so no session survived it: {error}"
                );
                return 0;
            }
        };
        let mut taken = 0;
        for one in carried {
            let mut tally = one.tally;
            // Never blank: assume what this console would have started it with.
            if tally.mode.is_none() {
                tally.mode = Some(
                    self.config
                        .spawn
                        .permission_mode
                        .clone()
                        .unwrap_or(crate::modes::Mode::Default),
                );
            }
            match Session::adopt(
                one.id.clone(),
                one.dir.clone().into(),
                one.pid,
                one.fds,
                tally,
            ) {
                Ok(session) => {
                    tracing::info!("carried {} (pid {}) across the upgrade", one.id, one.pid);
                    self.follow_edits(&session);
                    self.sessions.write().insert(one.id, session);
                    taken += 1;
                }
                Err(error) => {
                    tracing::error!("{} did not survive the upgrade: {error:#}", one.id);
                }
            }
        }
        taken
    }

    /// Send the kills the previous image owed, on its deadlines, or upgrades in a
    /// row would postpone them forever.
    pub fn finish_stopping(&self) -> usize {
        let Ok(handed) = std::env::var(STOPPING) else {
            return 0;
        };
        // So the next upgrade does not aim at a pid already dealt with.
        unsafe { std::env::remove_var(STOPPING) };
        let stopping: Vec<Stopping> = match serde_json::from_str(&handed) {
            Ok(stopping) => stopping,
            Err(error) => {
                tracing::error!("the stopping handover could not be read: {error}");
                return 0;
            }
        };
        let waiting = stopping.len();
        for one in stopping {
            let left = u64::try_from(one.due - crate::session::now()).unwrap_or(0);
            tracing::info!(
                "{} was being stopped — finishing it in {left}ms (pid {})",
                one.id,
                one.pid
            );
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(left)).await;
                // Which confirms the pid first.
                crate::session::finish(one.pid, &one.id);
            });
        }
        waiting
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Start a session in `dir`, which must be one the config allows.
    pub fn start(&self, dir: &str) -> Result<Arc<Session>, String> {
        let real = self.config.resolve(dir).inspect_err(|why| {
            tracing::warn!("refused a session in {dir}: {why}");
        })?;
        let id = uuid::Uuid::new_v4().to_string();
        tracing::info!("starting {id} in {}", real.display());
        let session = self.hold(
            id.clone(),
            Session::start(id.clone(), &real, &self.config.spawn),
        )?;
        // From the first spawn, so any later resume keeps the mode.
        if let Some(mode) = session.mode() {
            self.modes.set(&id, &mode);
        }
        Ok(session)
    }

    /// Pick a conversation back up with its id and mode — refused while anything runs
    /// it, so a transcript never has two writers. The mode comes from the session in
    /// hand, else [`crate::modes`].
    pub fn resume(&self, dir: &str, id: &str) -> Result<Arc<Session>, String> {
        let known = self
            .get(id)
            .and_then(|held| held.mode())
            .or_else(|| self.modes.get(id));
        self.resume_as(dir, id, known)
    }

    /// The same, with the mode stated outright — `None` for the console's configured
    /// one. See [`Self::revive`].
    fn resume_as(
        &self,
        dir: &str,
        id: &str,
        mode: Option<crate::modes::Mode>,
    ) -> Result<Arc<Session>, String> {
        let real = self.config.resolve(dir).inspect_err(|why| {
            tracing::warn!("refused a resume of {id} in {dir}: {why}");
        })?;
        if self.get(id).is_some_and(|session| session.alive()) {
            tracing::info!("refused {id}: this console already has it open");
            return Err(format!("{id} is already open here"));
        }
        // Refused when anything else appears to run it — `past::in_use`.
        if crate::past::conversations(&crate::past::projects_root())
            .iter()
            .any(|conversation| conversation.id == id && conversation.busy)
        {
            // Logged: the phone shows only one sentence.
            tracing::info!("refused {id}: past::in_use says something is already there");
            return Err(format!(
                "{id} looks like it is still in use — close it first. Two processes \
                 on one transcript both append, and neither sees the other's turns."
            ));
        }
        // The conversation's name becomes `-n`, its only route to peers — [`Spawn::name`].
        let spawn = crate::session::Spawn {
            permission_mode: mode
                .clone()
                .or_else(|| self.config.spawn.permission_mode.clone()),
            name: crate::past::named(&crate::past::projects_root(), id),
            ..self.config.spawn.clone()
        };
        // Logged: a resume can change the mode unseen.
        tracing::info!(
            "resuming {id} in {} on {}",
            real.display(),
            spawn
                .permission_mode
                .as_ref()
                .unwrap_or(&crate::modes::Mode::Default)
                .name()
        );
        let session = self.hold(
            id.to_string(),
            Session::resume(id.to_string(), &real, &spawn),
        )?;
        // Remembered from the first resume on.
        if let Some(mode) = session.mode() {
            self.modes.set(id, &mode);
        }
        Ok(session)
    }

    /// Restart a session that stopped listening, on the same conversation and mode,
    /// and re-send what it never read — that sat in the old process's pipe.
    pub async fn revive(&self, id: &str) -> Result<Arc<Session>, String> {
        let old = self
            .get(id)
            .ok_or_else(|| format!("{id} is not open on this console"))?;
        let dir = old.dir.display().to_string();
        let mode = old.mode();
        let unread = old.unread();
        tracing::info!(
            "reviving {id}: {} message(s) to re-send afterwards",
            unread.len()
        );
        // Only if running: `stop` arms a kill that would hit a reused pid.
        if old.alive() {
            old.stop().await;
        }
        // Generous: the kill comes after a grace period.
        let gone = std::time::Instant::now();
        while old.alive() && gone.elapsed() < REVIVE_PATIENCE {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        if old.alive() {
            return Err(format!(
                "{id} would not stop within {}s, so it has not been restarted — \
                 nothing has been lost, but it needs looking at by hand",
                REVIVE_PATIENCE.as_secs()
            ));
        }
        let fresh = self.resume_as(&dir, id, mode)?;
        for text in unread {
            if let Err(why) = fresh.send(&text).await {
                tracing::warn!("could not re-send a message to the revived {id}: {why:#}");
            }
        }
        Ok(fresh)
    }

    fn hold(
        &self,
        id: String,
        started: anyhow::Result<Arc<Session>>,
    ) -> Result<Arc<Session>, String> {
        let session = started
            .inspect_err(|err| tracing::error!("could not start {id}: {err:#}"))
            .map_err(|err| format!("{err:#}"))?;
        self.follow_edits(&session);
        self.sessions.write().insert(id, session.clone());
        Ok(session)
    }

    /// Check a backgrounded call's files once its task ends — [`crate::edits::Edits::ended`].
    fn follow_edits(&self, session: &Arc<Session>) {
        let edits = self.edits();
        let mut events = session.listen();
        let held = Arc::downgrade(session);
        tokio::spawn(async move {
            loop {
                let (call, status) = match events.recv().await {
                    Ok(Stamped {
                        event:
                            Event::Background {
                                tool: Some(call),
                                status,
                                ..
                            },
                        ..
                    }) => (call, status),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let edits = Arc::clone(&edits);
                let checked =
                    tokio::task::spawn_blocking(move || edits.ended(&call, status.as_ref()))
                        .await
                        .ok()
                        .flatten();
                if let Some((_, diverged)) = checked
                    && !diverged.paths.is_empty()
                    && let Some(session) = held.upgrade()
                {
                    session.diverged(diverged);
                }
            }
        });
    }

    /// Kill every session, for shutdown: an orphan would make its conversation look
    /// busy to the next console.
    pub fn shut_down(&self) {
        for session in self.sessions.read().values() {
            tracing::info!("killing {}", session.id);
            session.force();
        }
    }

    /// Replace this console with a newer build by `execve`, which keeps the pid and
    /// the children's pipes; each session travels in [`HANDOVER`], each being stopped
    /// in [`STOPPING`]. Returning means it failed and nothing was lost.
    pub fn handover(&self) -> anyhow::Result<std::convert::Infallible> {
        use std::os::unix::process::CommandExt;

        let sessions = self.sessions.read();
        let carried: Vec<Carried> = sessions
            .values()
            .filter(|session| session.alive())
            .filter(|session| {
                let fds = session.fds();
                // All three pipes or none.
                [fds.stdin, fds.stdout, fds.stderr]
                    .into_iter()
                    .all(crate::session::keepable)
            })
            .map(|session| Carried {
                id: session.id.clone(),
                dir: session.dir.display().to_string(),
                pid: session.pid(),
                fds: session.fds(),
                tally: session.tally(),
            })
            .collect();

        // Everything stopped and not yet gone.
        let stopping: Vec<Stopping> = sessions
            .values()
            .filter(|session| session.alive())
            .filter_map(|session| {
                session.stopping().map(|due| Stopping {
                    id: session.id.clone(),
                    pid: session.pid(),
                    due,
                })
            })
            .collect();

        let binary = std::env::current_exe().context("finding this console's own binary")?;
        tracing::info!(
            "upgrading to {}, carrying {} session(s) and {} being stopped",
            binary.display(),
            carried.len(),
            stopping.len()
        );
        let error = std::process::Command::new(&binary)
            // dev-lint: allow-argv-hand-parsed the new image gets this one's argv verbatim
            .args(std::env::args().skip(1))
            .env(HANDOVER, serde_json::to_string(&carried)?)
            .env(STOPPING, serde_json::to_string(&stopping)?)
            .exec();
        // exec only returns on failure.
        Err(anyhow::Error::new(error).context(format!("replacing this console with {binary:?}")))
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().get(id).cloned()
    }

    /// Ask one live session what the account has spent — the freshest idle one, since
    /// a busy CLI answers only after its turn. The answer reaches [`Self::spent`].
    pub async fn ask_usage(&self) {
        let asked = {
            let sessions = self.sessions.read();
            sessions
                .values()
                .filter(|session| session.alive())
                .max_by_key(|session| {
                    crate::usage::asked_before(session.working(), session.last_heard())
                })
                .cloned()
        };
        if let Some(session) = asked {
            session.ask_usage().await;
        }
    }

    /// What the API last said about each rate-limit window, across every session.
    pub fn spent(&self) -> std::collections::BTreeMap<String, crate::session::Seen> {
        let sessions = self.sessions.read();
        // Merged, not latest-wins: an idle session answers from its cache —
        // [`crate::usage::fresher`].
        let mut newest = self.spent.lock();
        for session in sessions.values() {
            crate::usage::remember(&mut newest, session.tally().spent);
        }
        newest.clone()
    }

    /// Notice sessions that stopped reading stdin, once per episode, and capture them
    /// before any restart. Swept: deafness is the absence of events.
    pub async fn watch_for_deafness(&self) {
        let live: Vec<_> = {
            let sessions = self.sessions.read();
            sessions
                .values()
                .filter(|session| session.alive())
                .cloned()
                .collect()
        };
        let root = crate::past::projects_root();
        for session in live {
            // Two minutes past its deadline, where the notice normally arrives.
            session.end_overdue(120_000);
            session.take_up_decision();
            let Some((seconds, unread)) = session.check_deaf() else {
                continue;
            };
            tracing::warn!(
                "{} has not read {unread} message(s) in {seconds}s — capturing before it is cured",
                session.id
            );
            // UTC, like every stamp this console writes.
            let stamp = time::OffsetDateTime::now_utc()
                .format(&time::macros::format_description!(
                    "[year]-[month]-[day]-[hour][minute][second]Z"
                ))
                .unwrap_or_else(|_| "deaf".to_string());
            crate::deaf::capture(
                &crate::deaf::evidence_root(),
                &session.id,
                session.pid(),
                crate::past::transcript_of(&root, &session.id).as_deref(),
                &stamp,
            )
            .await;
        }
    }

    /// A session's summary with what only its transcript and process know: name, last
    /// move, size, peer name. Every answer about a session is this one.
    pub fn summary_of(&self, session: &Session) -> Summary {
        let mut summary = session.summary();
        // See [`crate::past::about`] and [`crate::past::last_moved`].
        if let Some(about) = crate::past::about(&crate::past::projects_root(), &summary.id) {
            summary.name = about.name;
            summary.touched = Some(about.touched);
            summary.bytes = Some(about.bytes);
        }
        // By pid: the peer name belongs to the process.
        summary.peer_name = crate::peers::named(&crate::peers::sessions_root(), session.pid());
        summary
    }

    pub fn list(&self) -> Vec<Summary> {
        let sessions = self.sessions.read();
        let mut all: Vec<Summary> = sessions
            .values()
            .map(|session| self.summary_of(session))
            .collect();
        // By last activity, else pickup time (`started` is seconds, `touched` ms).
        all.sort_by_key(|session| {
            std::cmp::Reverse(session.touched.unwrap_or(session.started * 1000))
        });
        all
    }

    /// Forget a session, killing it first if it is live.
    pub fn forget(&self, id: &str) -> bool {
        let Some(session) = self.sessions.write().remove(id) else {
            tracing::info!("asked to forget {id}, which this console does not have");
            return false;
        };
        tracing::info!("forgetting {id} — killing it first if it is still running");
        session.force();
        // And its landmarks, the largest thing kept.
        self.marks.forget(id);
        true
    }
}

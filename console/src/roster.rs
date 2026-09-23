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
use crate::session::{Session, Summary};

/// How long [`Roster::revive`] will wait for a stopped session to actually go. A
/// stop kills only after a grace period, and one session took thirty seconds to
/// leave the process table, where the resume guard can still see it.
const REVIVE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(90);

pub struct Roster {
    config: Config,
    sessions: RwLock<BTreeMap<String, Arc<Session>>>,
    /// The account's rate-limit figure — see [`crate::usage`]. Held here so the front
    /// page reads it from memory.
    usage: Arc<crate::usage::Usage>,
    /// What each conversation is about, in a sentence. See [`crate::gist`].
    gists: Arc<crate::gist::Gists>,
    drafts: Arc<crate::drafts::Drafts>,
    /// How much is left of each session's task list, kept between sweeps. See
    /// [`crate::tasks::Tasks`].
    tasks: Arc<crate::tasks::Tasks>,
    /// What each conversation was last allowed to do without asking. See
    /// [`crate::modes`] — the only record of it anywhere.
    modes: Arc<crate::modes::Modes>,
    /// Each transcript's landmarks, walked once and then only extended. See
    /// [`crate::marks`] — the walk is the whole of the "go to" sheet's wait.
    marks: Arc<crate::marks::Marks>,
    /// The truest reading of each rate-limit window, kept across the sessions that
    /// heard it, or the figure steps back when one ends. Utilisation only rises inside
    /// a window, and
    /// [`crate::usage::fresher`] discards an old window outright.
    spent: Mutex<BTreeMap<String, crate::session::Seen>>,
}

/// The environment variable an upgrade hands its sessions over in.
const HANDOVER: &str = "CONSOLE_HANDOVER";

/// And the one for the sessions it was in the middle of stopping. A second
/// variable, so an upgrade from an older build is an absent variable rather than
/// a shape it cannot parse.
const STOPPING: &str = "CONSOLE_HANDOVER_STOPPING";

/// One session, as it travels across an upgrade.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Carried {
    id: String,
    dir: String,
    pid: u32,
    fds: crate::session::Fds,
    /// The counters, which are on no disk anywhere — see [`crate::session::Tally`].
    /// Defaulted so a handover from an older image still reads.
    #[serde(default)]
    tally: crate::session::Tally,
}

/// A session that was stopped, and whose kill is still owed to it. Carried apart
/// from the live ones: its stdin is already closed, which makes it invisible to
/// the handover, and all the new image can do is finish it off on time.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Stopping {
    id: String,
    pid: u32,
    /// When the kill falls due, in epoch milliseconds — the original deadline, not a
    /// fresh grace period.
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
            spent: Mutex::new(BTreeMap::new()),
        }
    }

    /// Every landmark in a conversation, walking only what has arrived since the
    /// last time somebody asked. See [`crate::marks`].
    pub fn marks(&self) -> Arc<crate::marks::Marks> {
        Arc::clone(&self.marks)
    }

    /// Remember what a conversation is allowed to do, so a later resume can put
    /// it back. See [`crate::modes`].
    pub fn remember_mode(&self, id: &str, mode: &str) {
        self.modes.set(id, mode);
    }

    /// Who is holding what, for the front page. Over the network now, cached for
    /// thirty seconds — see [`crate::tasks::Tasks`] — so an ordinary await.
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

    /// The unsent words each conversation is holding, so a client that has just
    /// connected learns which have one without asking per session.
    pub fn drafts(&self) -> Arc<crate::drafts::Drafts> {
        Arc::clone(&self.drafts)
    }

    /// Write a sentence for every conversation that has moved since its last
    /// one. Called on a timer from `main`; see [`crate::gist::Gists::sweep`].
    pub async fn write_gists(&self) {
        self.gists
            .sweep(&self.config.spawn.binary, &crate::past::projects_root())
            .await;
    }

    /// Drop the pictures whose conversations are gone; see [`crate::images::tidy`] for
    /// what it refuses to do. Off the runtime: it walks two trees and deletes.
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

    /// The rate-limit reading, for the front page and for the watcher that
    /// keeps it current.
    pub fn usage(&self) -> &Arc<crate::usage::Usage> {
        &self.usage
    }

    /// Pick up the sessions an upgrade handed over, if this image was exec'd by one.
    /// See [`Self::handover`].
    ///
    /// A session that cannot be rebuilt is dropped rather than guessed at: writing an
    /// instruction into a descriptor that was another session's would be worse than
    /// an unreachable process. Also finishes off what the old image was stopping —
    /// [`Self::finish_stopping`].
    pub fn inherit(&self) -> usize {
        let Ok(handed) = std::env::var(HANDOVER) else {
            return 0;
        };
        // Not left lying around: a session started later must not think it was inherited.
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
            // Never blank about permissions: an empty mode on the header reads as the careful
            // setting, which is the one case it might not be. What this console would have
            // started the session with is the best answer available.
            if tally.mode.is_none() {
                tally.mode = Some(
                    self.config
                        .spawn
                        .permission_mode
                        .clone()
                        .unwrap_or_else(|| crate::session::DEFAULT_MODE.to_string()),
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

    /// Send the kills the image before this one promised and could not deliver.
    ///
    /// The deadline is the old image's: a session stopped twenty-nine seconds before
    /// an upgrade gets one second, or enough upgrades in a row are the leak this
    /// fixes. Each waits on its own task; the log is where this reports.
    pub fn finish_stopping(&self) -> usize {
        let Ok(handed) = std::env::var(STOPPING) else {
            return 0;
        };
        // Removed so the next upgrade does not aim a kill at a pid dealt with already.
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
                // Which checks the pid is still that conversation before sending anything.
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
        // From the very first spawn, so a resume after this console is gone comes back on
        // the mode it was started with. See [`crate::modes`].
        if let Some(mode) = session.mode() {
            self.modes.set(&id, &mode);
        }
        Ok(session)
    }

    /// Pick an existing conversation back up, keeping its id and its mode.
    ///
    /// Refused when this console is already running it: two processes appending to
    /// one transcript is a mess with no clean end. It cannot see a `claude` in a
    /// terminal, so the guard is a rail, not a boundary.
    ///
    /// The mode comes from the session still in hand, else from [`crate::modes`] —
    /// the case that matters, since it is an ended session that gets resumed, and the
    /// console's default would drop it to Manual.
    pub fn resume(&self, dir: &str, id: &str) -> Result<Arc<Session>, String> {
        let known = self
            .get(id)
            .and_then(|held| held.mode())
            .or_else(|| self.modes.get(id));
        self.resume_as(dir, id, known)
    }

    /// The same, with the mode stated outright — `None` for the console's configured
    /// one. See [`Self::revive`].
    fn resume_as(&self, dir: &str, id: &str, mode: Option<String>) -> Result<Arc<Session>, String> {
        let real = self.config.resolve(dir).inspect_err(|why| {
            tracing::warn!("refused a resume of {id} in {dir}: {why}");
        })?;
        if self.get(id).is_some_and(|session| session.alive()) {
            tracing::info!("refused {id}: this console already has it open");
            return Err(format!("{id} is already open here"));
        }
        // And refused when anything ELSE appears to be using it; as a warning in the UI
        // it let a second process onto a transcript. `busy` is inferred — see
        // `past::in_use`.
        if crate::past::conversations(&crate::past::projects_root())
            .iter()
            .any(|conversation| conversation.id == id && conversation.busy)
        {
            // Logged with its reason, since from the phone a refusal is one sentence. It
            // means a running `claude` names this conversation, or `ps` could not be asked;
            // `past::arguments` warns for the second.
            tracing::info!("refused {id}: past::in_use says something is already there");
            return Err(format!(
                "{id} looks like it is still in use — close it first. Two processes \
                 on one transcript both append, and neither sees the other's turns."
            ));
        }
        // The conversation's own name becomes `-n`, because that is the only route by
        // which it reaches the peer registry: renaming a running session writes a title
        // to the transcript and nothing else, so a name given through this console was
        // invisible to every other session until its next resume. See [`Spawn::name`].
        let spawn = crate::session::Spawn {
            permission_mode: mode
                .clone()
                .or_else(|| self.config.spawn.permission_mode.clone()),
            name: crate::past::named(&crate::past::projects_root(), id),
            ..self.config.spawn.clone()
        };
        // Logged: the mode is what a resume can change without anyone seeing.
        tracing::info!(
            "resuming {id} in {} on {}",
            real.display(),
            spawn
                .permission_mode
                .as_deref()
                .unwrap_or(crate::session::DEFAULT_MODE)
        );
        let session = self.hold(
            id.to_string(),
            Session::resume(id.to_string(), &real, &spawn),
        )?;
        // Remembered on the way out as well, so a conversation this console never held
        // before is known from its first resume.
        if let Some(mode) = session.mode() {
            self.modes.set(id, &mode);
        }
        Ok(session)
    }

    /// Stop a session that has stopped listening, start it again on the same
    /// conversation, and give it back what it never read.
    ///
    /// The only known cure, and not a repair. The unread messages are re-sent by hand,
    /// since they sit in the old process's pipe — the step somebody doing it manually
    /// forgets. The mode is carried across; a cure that takes a session's permissions
    /// away is one people learn not to use.
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
        // Only if it is still running: `stop` arms a kill thirty seconds out, which for a
        // session already gone is a SIGKILL at a pid this console no longer owns.
        if old.alive() {
            old.stop().await;
        }
        // Bounded, generously: a stop kills only after a grace period, and a session has
        // taken thirty seconds to go, passing through `Z` where the resume guard sees it.
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
        self.sessions.write().insert(id, session.clone());
        Ok(session)
    }

    /// Kill every session this console owns, for shutdown. Kill, not a polite stop:
    /// `kill_on_drop` covers a clean exit only, and orphans keep their session ids
    /// and make their conversations look busy to the next console.
    pub fn shut_down(&self) {
        for session in self.sessions.read().values() {
            tracing::info!("killing {}", session.id);
            session.force();
        }
    }

    /// Replace this console with a newer build, keeping every session alive.
    ///
    /// `execve` keeps the pid and every descriptor not close-on-exec, so the `claude`
    /// children never notice. Per session, id, directory, pid and three descriptor
    /// numbers travel as JSON in [`HANDOVER`]; the listening sockets deliberately do
    /// not, so the port is free at once and clients reconnect on `Last-Event-ID`.
    ///
    /// If this RETURNS, the upgrade failed and this is still the old build, holding
    /// everything it held. A session being stopped travels in [`STOPPING`]: it fails
    /// the descriptor test by construction, and its kill lives in a task `execve`
    /// discards.
    pub fn handover(&self) -> anyhow::Result<std::convert::Infallible> {
        use std::os::unix::process::CommandExt;

        let sessions = self.sessions.read();
        let carried: Vec<Carried> = sessions
            .values()
            .filter(|session| session.alive())
            .filter(|session| {
                let fds = session.fds();
                // All three or none: a session missing a pipe would be a conversation on screen
                // that answers nothing.
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

        // Everything stopped and not yet gone. Not filtered on the descriptors: closing
        // stdin is what put it here.
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

    /// Ask a live session what the account has spent. One session — the figure is
    /// account-wide — and the freshest IDLE one: a busy CLI answers no control request
    /// until its turn ends, and "spoke most recently" nearly defines "working now".
    /// `(not working, last heard)` falls back to the freshest working one. Nothing is
    /// returned; the answer lands in that session's tally, where [`Self::spent`] finds it.
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

    /// What the API has most recently said about each rate-limit window, across every
    /// session: the session that heard it is an accident of which one was working.
    pub fn spent(&self) -> std::collections::BTreeMap<String, crate::session::Seen> {
        let sessions = self.sessions.read();
        // Merged into what is remembered, not gathered afresh, and not "whichever
        // arrived last" — see [`crate::usage::fresher`]: an idle session answers from
        // its cache, so the freshest arrival is routinely the oldest figure.
        let mut newest = self.spent.lock();
        for session in sessions.values() {
            crate::usage::remember(&mut newest, session.tally().spent);
        }
        newest.clone()
    }

    /// Notice sessions that have stopped reading their stdin, and write down what they
    /// look like before anybody restarts them. Swept rather than pushed: deafness is
    /// the absence of events. See [`crate::session::Session::deaf`] and [`crate::deaf`].
    /// Announced once per episode.
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
            let Some((seconds, unread)) = session.check_deaf() else {
                continue;
            };
            tracing::warn!(
                "{} has not read {unread} message(s) in {seconds}s — capturing before it is cured",
                session.id
            );
            // UTC and named so, like every stamped file this console writes.
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

    pub fn list(&self) -> Vec<Summary> {
        let sessions = self.sessions.read();
        // The name and the last-activity time are the transcript's, not the session's,
        // so the roster reads them here in one pass over the tail and metadata.
        let root = crate::past::projects_root();
        let peers = crate::peers::sessions_root();
        let mut all: Vec<Summary> = sessions
            .values()
            .map(|session| {
                let mut summary = session.summary();
                // See [`crate::past::about`], and [`crate::past::last_moved`] for why the date is
                // read out of the conversation rather than off the file.
                if let Some(about) = crate::past::about(&root, &summary.id) {
                    summary.name = about.name;
                    summary.touched = Some(about.touched);
                    summary.bytes = Some(about.bytes);
                }
                // The other name, read by pid rather than by conversation — it belongs to
                // the process, not the transcript. See [`Summary::peer_name`].
                summary.peer_name = crate::peers::named(&peers, session.pid());
                summary
            })
            .collect();
        // By last activity, falling back to pickup time: `started` is seconds, `touched`
        // milliseconds.
        all.sort_by_key(|session| {
            std::cmp::Reverse(session.touched.unwrap_or(session.started * 1000))
        });
        all
    }

    /// Forget an ended session. A live one is killed first, or an agent would be
    /// working with nothing holding its handle.
    pub fn forget(&self, id: &str) -> bool {
        let Some(session) = self.sessions.write().remove(id) else {
            tracing::info!("asked to forget {id}, which this console does not have");
            return false;
        };
        tracing::info!("forgetting {id} — killing it first if it is still running");
        session.force();
        // And its landmarks, the largest thing kept per conversation.
        self.marks.forget(id);
        true
    }
}

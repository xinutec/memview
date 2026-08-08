//! Every session the console owns.
//!
//! Sessions stay listed after they end. A session that failed to start, or that
//! exited on its own, is the thing most worth seeing — dropping it from the list
//! the moment it dies would leave a client that looked a second too late with no
//! evidence that anything ever happened.

use anyhow::Context as _;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;

use crate::config::Config;
use crate::session::{Session, Summary};

/// How long [`Roster::revive`] will wait for a stopped session to actually go.
///
/// A stop closes stdin and kills only after a grace period, and one measured
/// session took about thirty seconds to leave the process table — where the
/// resume guard can still see it. Bounded rather than patient for ever, because
/// the caller is somebody holding a phone.
const REVIVE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(90);

pub struct Roster {
    config: Config,
    sessions: RwLock<BTreeMap<String, Arc<Session>>>,
    /// The account's rate-limit figure, fetched rather than measured — see
    /// [`crate::usage`]. Held here so the front page reads it from memory.
    usage: Arc<crate::usage::Usage>,
    /// What each conversation is about, in a sentence. See [`crate::gist`].
    gists: Arc<crate::gist::Gists>,
    /// How much is left of each session's task list, kept between sweeps. See
    /// [`crate::tasks::Tasks`].
    tasks: Arc<crate::tasks::Tasks>,
    /// What each conversation was last allowed to do without asking. See
    /// [`crate::modes`] — the only record of it anywhere.
    modes: Arc<crate::modes::Modes>,
}

/// The environment variable an upgrade hands its sessions over in.
const HANDOVER: &str = "CONSOLE_HANDOVER";

/// One session, as it travels across an upgrade.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Carried {
    id: String,
    dir: String,
    pid: u32,
    fds: crate::session::Fds,
    /// The counters, which unlike the conversation are on no disk anywhere —
    /// see [`crate::session::Tally`]. Defaulted so a handover written by an
    /// older image is still readable rather than dropping every session.
    #[serde(default)]
    tally: crate::session::Tally,
}

impl Roster {
    pub fn new(config: Config) -> Self {
        let usage = Arc::new(crate::usage::Usage::new(config.usage_url.clone()));
        let gists = Arc::new(crate::gist::Gists::load(config.gists.clone()));
        let modes = Arc::new(crate::modes::Modes::load(config.modes.clone()));
        Self {
            config,
            sessions: RwLock::new(BTreeMap::new()),
            usage,
            gists,
            tasks: Arc::default(),
            modes,
        }
    }

    /// Remember what a conversation is allowed to do, so a later resume can put
    /// it back. See [`crate::modes`].
    pub fn remember_mode(&self, id: &str, mode: &str) {
        self.modes.set(id, mode);
    }

    /// How much is left of each session's task list, for the front page.
    ///
    /// ⚠ **Over the network now, where this was a sweep of a directory.** It
    /// used to be `spawn_blocking`, because reading every session's task files
    /// cold was seconds of I/O and would have stalled every stream on the
    /// executor. The service answers in 56-139 ms and the answer is cached for
    /// thirty seconds — see [`crate::tasks::Tasks`] — so this is now an ordinary
    /// await that is usually not a request at all.
    pub async fn tasks(&self) -> BTreeMap<String, crate::tasks::Count> {
        self.tasks.sweep().await
    }

    /// One conversation's tasks, for the sheet that lists them.
    pub async fn task_list(&self, session: &str) -> Vec<crate::tasks::Listed> {
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

    /// Write a sentence for every conversation that has moved since its last
    /// one. Called on a timer from `main`; see [`crate::gist::Gists::sweep`].
    pub async fn write_gists(&self) {
        self.gists
            .sweep(&self.config.spawn.binary, &crate::past::projects_root())
            .await;
    }

    /// Drop the pictures whose conversations are gone. Called on a timer from
    /// `main`; see [`crate::images::tidy`] for what it refuses to do.
    ///
    /// Off the runtime, like [`Self::tasks`]: this walks two directory trees and
    /// may delete from one of them, and neither is work to do on a thread that is
    /// meant to be answering requests.
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

    /// Pick up the sessions an upgrade handed over, if this image was exec'd by
    /// one. See [`Self::handover`].
    ///
    /// A session that cannot be rebuilt is *dropped rather than guessed at*: its
    /// process is still running and now unreachable, which is bad, but writing
    /// somebody's next instruction into a descriptor that turned out to be a
    /// different session's would be worse. The number carried alongside each
    /// descriptor is the id, so a mismatch is at least visible in the log.
    pub fn inherit(&self) -> usize {
        let Ok(handed) = std::env::var(HANDOVER) else {
            return 0;
        };
        // Not left lying around: a session started later must not think it was
        // inherited, and the numbers in here mean nothing once used.
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
            // ⚠ **Never blank about permissions.** A handover written by an image
            // that did not carry the mode leaves this empty, and an empty mode on
            // the header reads as the careful setting — which is the one case it
            // might not be. What this console would have started the session with
            // is the best answer available, and it is right whenever the session
            // was started by a console configured as this one is.
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
                    self.sessions
                        .write()
                        .expect("roster poisoned")
                        .insert(one.id, session);
                    taken += 1;
                }
                Err(error) => {
                    tracing::error!("{} did not survive the upgrade: {error:#}", one.id);
                }
            }
        }
        taken
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
        // From the very first spawn, so that a conversation resumed after this
        // console is gone comes back on the mode it was actually started with
        // rather than on whatever the default is by then. See [`crate::modes`].
        if let Some(mode) = session.mode() {
            self.modes.set(&id, &mode);
        }
        Ok(session)
    }

    /// Pick up an existing conversation, keeping its id.
    ///
    /// Refuses one this console is already running, because two processes
    /// appending to one transcript is a mess with no clean end — each writes
    /// turns the other does not know about. It cannot refuse a `claude` in a
    /// terminal, which the console has no way to see, so the guard is a rail and
    /// not a boundary.
    /// Pick a conversation back up on the mode it was last on.
    ///
    /// ⚠ **Resuming used to drop a session to Manual without saying so.**
    /// Measured 2026-08-08: `hardware` was running in `auto`, was stopped and
    /// resumed, and came back `default` — and the console reported that as
    /// though it had always been the mode. A session left in `auto` was left
    /// that way because nobody is watching it, so Manual means it stops at the
    /// first tool call needing approval and waits, which from a phone looks
    /// exactly like the stall that prompted the restart (memview #119).
    ///
    /// The mode comes from the session still in hand if there is one, and from
    /// [`crate::modes`] otherwise — which is the case that matters, because a
    /// console upgrade drops ended sessions and it is precisely a session that
    /// has ended that somebody is resuming.
    pub fn resume(&self, dir: &str, id: &str) -> Result<Arc<Session>, String> {
        let known = self
            .get(id)
            .and_then(|held| held.mode())
            .or_else(|| self.modes.get(id));
        self.resume_as(dir, id, known)
    }

    /// The same, with the mode to bring the session back on stated outright —
    /// `None` for the console's configured one. See [`Self::revive`].
    fn resume_as(&self, dir: &str, id: &str, mode: Option<String>) -> Result<Arc<Session>, String> {
        let real = self.config.resolve(dir).inspect_err(|why| {
            tracing::warn!("refused a resume of {id} in {dir}: {why}");
        })?;
        if self.get(id).is_some_and(|session| session.alive()) {
            tracing::info!("refused {id}: this console already has it open");
            return Err(format!("{id} is already open here"));
        }
        // And refused when anything *else* appears to be using it. This is the
        // guard that was a warning in the UI first, which is not a guard: it let
        // a second process onto a transcript a remote-controlled session was
        // still writing. `busy` is inferred rather than reported — see
        // `past::in_use` for why there is nothing to report it.
        if crate::past::conversations(&crate::past::projects_root())
            .iter()
            .any(|conversation| conversation.id == id && conversation.busy)
        {
            // Logged with its own reason, because from the phone a refusal is one
            // sentence with nothing behind it. The check has two arms — a process
            // that names the conversation, and a transcript written moments ago —
            // and which one fired is the whole difference between "wait a minute"
            // and "close the other window". This is where that survives.
            tracing::info!("refused {id}: past::in_use says something is already there");
            return Err(format!(
                "{id} looks like it is still in use — close it first. Two processes \
                 on one transcript both append, and neither sees the other's turns."
            ));
        }
        let spawn = match &mode {
            Some(mode) => crate::session::Spawn {
                permission_mode: Some(mode.clone()),
                ..self.config.spawn.clone()
            },
            None => self.config.spawn.clone(),
        };
        // Said out loud, because it is the one thing about a resume that used to
        // change silently.
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
        // Remembered on the way out as well as in, so a conversation this
        // console has never held before is known from its first resume onward.
        if let Some(mode) = session.mode() {
            self.modes.set(id, &mode);
        }
        Ok(session)
    }

    /// Stop a session that has stopped listening, start it again on the same
    /// conversation, and give it back what it never read.
    ///
    /// **The only known cure**, and it is not a repair — nothing here fixes
    /// whatever stops the CLI draining its pipe. What it does keep is the part
    /// that matters: the id, the transcript and the conversation all survive, so
    /// the cost is the in-memory state and the wait.
    ///
    /// **The unread messages have to be re-sent by hand**, because they are in
    /// the old process's pipe and the old process is being killed. This is the
    /// step somebody doing it manually forgets, and then the session is answering
    /// a question nobody remembers asking.
    ///
    /// ⚠ **The mode is carried across deliberately.** A plain resume passes the
    /// console's configured mode and a session that was on `acceptEdits` comes
    /// back on `default`, asking permission for every call (memview #119). A cure
    /// that quietly takes a session's permissions away is a cure people learn not
    /// to use.
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
        // ⚠ **Only if it is still running.** `stop` arms a timer that kills the
        // pid thirty seconds later if it has not gone — and for a session that
        // has *already* gone, that is a SIGKILL sent to a pid this console no
        // longer owns, thirty seconds after a new process was started in its
        // place. Rare, and the kind of rare that is untraceable when it happens.
        if old.alive() {
            old.stop().await;
        }
        // Bounded, and the bound is generous on purpose: a stop closes stdin and
        // only kills after a grace period, and a session has been measured taking
        // about thirty seconds to go — passing through `Z` on the way, which is a
        // process the resume guard can still see.
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
        self.sessions
            .write()
            .expect("roster poisoned")
            .insert(id, session.clone());
        Ok(session)
    }

    /// Kill every session this console owns.
    ///
    /// For shutdown, and it has to be *kill* rather than a polite stop: the
    /// console is on its way out and has no time left to wait for a turn to
    /// finish. `kill_on_drop` covers a clean exit and nothing else — a signalled
    /// process never runs a destructor, and the children it leaves behind keep
    /// their session ids, their working directories, and their place in the
    /// process table, where they go on making their conversations look busy to
    /// the next console that starts.
    pub fn shut_down(&self) {
        for session in self.sessions.read().expect("roster poisoned").values() {
            tracing::info!("killing {}", session.id);
            session.force();
        }
    }

    /// Replace this console with a newer build, keeping every session alive.
    ///
    /// `execve` replaces the image without touching the process: same pid, so
    /// the `claude` children are still children and never notice, and open
    /// descriptors survive unless they are close-on-exec. So the pipes to those
    /// children can be carried across — which is the whole feature, because
    /// stopping and starting instead kills every conversation and costs whoever
    /// is using it a re-entry.
    ///
    /// What travels is the minimum: for each session its id, directory, pid and
    /// three descriptor numbers, as JSON in [`HANDOVER`]. The new image rebuilds
    /// sessions from those rather than spawning any.
    ///
    /// ⚠ **The listening sockets are deliberately NOT carried.** They are
    /// close-on-exec and stay that way, so the port is free the instant the
    /// image is replaced and the new one binds it immediately. Clients see a
    /// dropped connection and reconnect quoting `Last-Event-ID`, which they
    /// already do for a train going through a cutting.
    ///
    /// ⚠ **If this returns, the upgrade failed and the console is still the old
    /// build**, holding everything it held before. It is written that way on
    /// purpose: the alternative — exiting on a failed exec — would leave live
    /// `claude` processes with nobody holding their stdin, reachable only by
    /// being killed.
    pub fn handover(&self) -> anyhow::Result<std::convert::Infallible> {
        use std::os::unix::process::CommandExt;

        let sessions = self.sessions.read().expect("roster poisoned");
        let carried: Vec<Carried> = sessions
            .values()
            .filter(|session| session.alive())
            .filter(|session| {
                let fds = session.fds();
                // All three or none: a session missing one of its pipes cannot
                // be spoken to or heard, and carrying it would produce a
                // conversation on screen that answers nothing.
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

        let binary = std::env::current_exe().context("finding this console's own binary")?;
        tracing::info!(
            "upgrading to {}, carrying {} session(s)",
            binary.display(),
            carried.len()
        );
        let error = std::process::Command::new(&binary)
            .args(std::env::args().skip(1))
            .env(HANDOVER, serde_json::to_string(&carried)?)
            .exec();
        // exec only returns on failure.
        Err(anyhow::Error::new(error).context(format!("replacing this console with {binary:?}")))
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions
            .read()
            .expect("roster poisoned")
            .get(id)
            .cloned()
    }

    /// Most recently active first, which is the order a console is read in.
    /// Ask a live session what the account has spent.
    ///
    /// ⚠ **One session, not all of them.** The figure is account-wide, so asking
    /// a second would be asking the same question twice and putting a line down
    /// a second conversation's stdin for an answer already known.
    ///
    /// ⚠ **The one that spoke most recently, not the one started most recently.**
    /// A session answers `get_usage` out of its own process's cached rate-limit
    /// headers, which are as old as that process's last request to the API — so
    /// asking a session that has been sitting idle since it was picked up gets a
    /// truthful answer about an hour ago. This used to choose by `started`, and
    /// four conversations resumed in one evening made the newest of them
    /// permanently the quietest: the figure on the phone flipped between now and
    /// an hour ago on this loop's own beat. See [`crate::usage::fresher`] for the
    /// half of the fix that survives being asked the wrong session anyway.
    ///
    /// Nothing is returned: the answer comes back on that session's stdout and
    /// lands in its tally, where [`Self::spent`] finds it.
    pub async fn ask_usage(&self) {
        let busiest = {
            let sessions = self.sessions.read().expect("roster poisoned");
            sessions
                .values()
                .filter(|session| session.alive())
                .max_by_key(|session| session.last_heard())
                .cloned()
        };
        if let Some(session) = busiest {
            session.ask_usage().await;
        }
    }

    /// What the API has most recently said about each rate-limit window.
    ///
    /// ⚠ **Across every session, keeping the newest per window.** The figure is
    /// account-wide — it comes off the response headers of whichever request was
    /// answered last — so the session that heard it is an accident of which one
    /// happened to be working. Taking any single session's copy would report the
    /// account as it stood when *that* conversation last did something, which
    /// for an idle one is hours ago.
    pub fn spent(&self) -> std::collections::BTreeMap<String, crate::session::Seen> {
        let sessions = self.sessions.read().expect("roster poisoned");
        let mut newest: std::collections::BTreeMap<String, crate::session::Seen> =
            std::collections::BTreeMap::new();
        for session in sessions.values() {
            for (window, seen) in session.tally().spent {
                // ⚠ Not "whichever arrived last" — see [`crate::usage::fresher`].
                // An idle session answers from its own process's cache, so the
                // freshest arrival is routinely the oldest figure.
                match newest.get(&window) {
                    Some(held) if !crate::usage::fresher(held, &seen) => {}
                    _ => {
                        newest.insert(window, seen);
                    }
                }
            }
        }
        newest
    }

    /// Notice sessions that have stopped reading their stdin, and write down
    /// what they look like before anybody restarts them.
    ///
    /// Swept rather than pushed, because deafness is the absence of events:
    /// nothing arrives to announce it, which is the whole difficulty. See
    /// [`crate::session::Session::deaf`] for the verdict and [`crate::deaf`] for
    /// what is captured.
    ///
    /// Each session is announced once per episode, so this can be run as often
    /// as the sharpness of the alarm is worth — the cost of a sweep that finds
    /// nothing is one comparison per session.
    pub async fn watch_for_deafness(&self) {
        let live: Vec<_> = {
            let sessions = self.sessions.read().expect("roster poisoned");
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
            // UTC and named so, like every other stamped file this console
            // writes — see the note in [`crate::api`].
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
        let sessions = self.sessions.read().expect("roster poisoned");
        // Neither the name nor the last-activity time is the session's to know —
        // the CLI writes both to the transcript and announces neither — so the
        // roster reads them here, one pass over the file's tail and metadata.
        let root = crate::past::projects_root();
        let mut all: Vec<Summary> = sessions
            .values()
            .map(|session| {
                let mut summary = session.summary();
                // One pass over the file for all three — see
                // [`crate::past::about`], and [`crate::past::last_moved`] for why
                // the date is read out of the conversation rather than off the
                // file.
                if let Some(about) = crate::past::about(&root, &summary.id) {
                    summary.name = about.name;
                    summary.touched = Some(about.touched);
                    summary.bytes = Some(about.bytes);
                }
                summary
            })
            .collect();
        // By last activity, falling back to when this console picked the session
        // up — a session with no transcript yet has nothing else to be ordered
        // by, and `started` is in seconds where `touched` is in milliseconds.
        all.sort_by_key(|session| {
            std::cmp::Reverse(session.touched.unwrap_or(session.started * 1000))
        });
        all
    }

    /// Forget an ended session. A live one is killed first: forgetting a session
    /// while its process runs would leave an agent working with nothing holding
    /// its handle.
    pub fn forget(&self, id: &str) -> bool {
        let Some(session) = self.sessions.write().expect("roster poisoned").remove(id) else {
            tracing::info!("asked to forget {id}, which this console does not have");
            return false;
        };
        tracing::info!("forgetting {id} — killing it first if it is still running");
        session.force();
        true
    }
}

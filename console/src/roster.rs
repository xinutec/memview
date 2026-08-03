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

pub struct Roster {
    config: Config,
    sessions: RwLock<BTreeMap<String, Arc<Session>>>,
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
        Self {
            config,
            sessions: RwLock::new(BTreeMap::new()),
        }
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
        self.hold(id.clone(), Session::start(id, &real, &self.config.spawn))
    }

    /// Pick up an existing conversation, keeping its id.
    ///
    /// Refuses one this console is already running, because two processes
    /// appending to one transcript is a mess with no clean end — each writes
    /// turns the other does not know about. It cannot refuse a `claude` in a
    /// terminal, which the console has no way to see, so the guard is a rail and
    /// not a boundary.
    pub fn resume(&self, dir: &str, id: &str) -> Result<Arc<Session>, String> {
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
        tracing::info!("resuming {id} in {}", real.display());
        self.hold(
            id.to_string(),
            Session::resume(id.to_string(), &real, &self.config.spawn),
        )
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

    /// Newest first, which is the order a console is read in.
    pub fn list(&self) -> Vec<Summary> {
        let sessions = self.sessions.read().expect("roster poisoned");
        // The name is not the session's to know — the CLI writes it to the
        // transcript and announces it nowhere — so the roster reads it here.
        let root = crate::past::projects_root();
        let mut all: Vec<Summary> = sessions
            .values()
            .map(|session| {
                let mut summary = session.summary();
                summary.name = crate::past::named(&root, &summary.id);
                summary
            })
            .collect();
        all.sort_by_key(|session| std::cmp::Reverse(session.started));
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

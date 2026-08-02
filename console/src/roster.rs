//! Every session the console owns.
//!
//! Sessions stay listed after they end. A session that failed to start, or that
//! exited on its own, is the thing most worth seeing — dropping it from the list
//! the moment it dies would leave a client that looked a second too late with no
//! evidence that anything ever happened.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;

use crate::config::Config;
use crate::session::{Session, Summary};

pub struct Roster {
    config: Config,
    sessions: RwLock<BTreeMap<String, Arc<Session>>>,
}

impl Roster {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            sessions: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Start a session in `dir`, which must be one the config allows.
    pub fn start(&self, dir: &str) -> Result<Arc<Session>, String> {
        let real = self.config.resolve(dir)?;
        let id = uuid::Uuid::new_v4().to_string();
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
        let real = self.config.resolve(dir)?;
        if self.get(id).is_some_and(|session| session.alive()) {
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
            return Err(format!(
                "{id} looks like it is still in use — close it first. Two processes \
                 on one transcript both append, and neither sees the other's turns."
            ));
        }
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
        let session = started.map_err(|err| format!("{err:#}"))?;
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
            session.force();
        }
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
        let mut all: Vec<Summary> = sessions.values().map(|s| s.summary()).collect();
        all.sort_by_key(|session| std::cmp::Reverse(session.started));
        all
    }

    /// Forget an ended session. A live one is killed first: forgetting a session
    /// while its process runs would leave an agent working with nothing holding
    /// its handle.
    pub fn forget(&self, id: &str) -> bool {
        let Some(session) = self.sessions.write().expect("roster poisoned").remove(id) else {
            return false;
        };
        session.force();
        true
    }
}

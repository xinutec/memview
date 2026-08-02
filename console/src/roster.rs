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
        let session = Session::start(id.clone(), &real, &self.config.spawn)
            .map_err(|err| format!("{err:#}"))?;
        self.sessions
            .write()
            .expect("roster poisoned")
            .insert(id, session.clone());
        Ok(session)
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

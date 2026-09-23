//! What each conversation was last allowed to do without asking.
//!
//! Nothing else records it: the mode goes to the CLI on the command line, and the
//! transcript's mode lines belong to whichever session was resumed (see
//! [`crate::session::Summary::mode`]). Without this file a session left in `auto`
//! came back Manual after a restart and stopped at its first approval, with
//! nothing saying why.

use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Every conversation's last known mode, by session id.
#[derive(Debug)]
pub struct Modes {
    store: PathBuf,
    held: RwLock<BTreeMap<String, String>>,
}

impl Modes {
    /// Read what the last run wrote. An unreadable file is an empty set and a loud log
    /// line: sessions then come back Manual, as they would with no file at all.
    pub fn load(store: PathBuf) -> Self {
        let held = match std::fs::read_to_string(&store) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(held) => held,
                Err(why) => {
                    tracing::error!(
                        "modes: {} will not parse ({why}) — every session will resume Manual",
                        store.display()
                    );
                    BTreeMap::new()
                }
            },
            // No file at all is the ordinary first run, and says nothing.
            Err(_) => BTreeMap::new(),
        };
        Self {
            store,
            held: RwLock::new(held),
        }
    }

    /// What this conversation was last allowed to do, if it has ever been said.
    pub fn get(&self, id: &str) -> Option<String> {
        self.held.read().get(id).cloned()
    }

    /// Remember a conversation's mode, if it has changed — this runs on every spawn
    /// and every mode request.
    pub fn set(&self, id: &str, mode: &str) {
        let all = {
            let mut held = self.held.write();
            if held.get(id).is_some_and(|known| known == mode) {
                return;
            }
            held.insert(id.to_string(), mode.to_string());
            held.clone()
        };
        self.write(&all);
    }

    /// Written whole each time — one short line per conversation, like
    /// [`crate::gist::Gists`], and a rewrite cannot leave a half-updated entry.
    fn write(&self, all: &BTreeMap<String, String>) {
        if let Ok(text) = serde_json::to_string_pretty(all)
            && let Some(dir) = self.store.parent()
        {
            let _ = std::fs::create_dir_all(dir);
            if let Err(why) = std::fs::write(&self.store, text) {
                tracing::warn!("modes: could not write {}: {why}", self.store.display());
            }
        }
    }
}

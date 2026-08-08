//! What each conversation was last allowed to do without asking.
//!
//! ⚠ **Nothing else records this, and that is the whole reason the file
//! exists.** The permission mode is passed to the CLI on the command line and
//! never written to the transcript in a form that can be trusted — a session
//! resumed from an interactive one carries *that* session's mode lines, so
//! reading them back reports the wrong answer with complete confidence (see
//! [`crate::session::Summary::mode`]). The console is the only thing that knows,
//! and until now it only knew for as long as it held the process.
//!
//! **What that cost.** Measured 2026-08-08 on `hardware`: a session left in
//! `auto` — deliberately, because nobody was watching it — was stopped and
//! resumed, and came back as `default`. Manual. It then stops at the first tool
//! call that needs approval and waits, which from a phone is indistinguishable
//! from the stall that prompted the restart. Nothing said the mode had changed;
//! the console reported the new one as though it had always been that.
//!
//! So the mode is remembered here, across a restart of the session, of the
//! console, and of the machine.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

/// Every conversation's last known mode, by session id.
#[derive(Debug)]
pub struct Modes {
    store: PathBuf,
    held: RwLock<BTreeMap<String, String>>,
}

impl Modes {
    /// Read what the last run wrote.
    ///
    /// An unreadable file is an empty set and a loud log line, not an error: a
    /// console that refuses to start because it cannot remember a preference is
    /// worse than one that starts and asks. The cost of the empty case is that
    /// sessions come back Manual — which is the behaviour this replaces, so it
    /// is a return to the old failure rather than a new one.
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
        self.held.read().expect("modes poisoned").get(id).cloned()
    }

    /// Remember a conversation's mode, if it has changed.
    ///
    /// Guarded on a change rather than written every time, because this is
    /// called on every spawn and every mode request, and the common case is that
    /// nothing is different — a rewrite per message would be a file write on a
    /// path that has no reason to touch the disk at all.
    pub fn set(&self, id: &str, mode: &str) {
        let all = {
            let mut held = self.held.write().expect("modes poisoned");
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

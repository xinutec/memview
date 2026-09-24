//! What each conversation was last allowed to do without asking.
//!
//! Nothing else records it: the mode goes to the CLI on the command line, and the
//! transcript's mode lines belong to whichever session was resumed (see
//! [`crate::session::Summary::mode`]). Without this file a session left in `auto`
//! came back Manual after a restart and stopped at its first approval, with
//! nothing saying why.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What a session may do without asking, in the CLI's own names. `Default` is
/// shown as *Manual*; the client keeps the label table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Mode {
    Plan,
    Default,
    DontAsk,
    AcceptEdits,
    Auto,
    BypassPermissions,
    /// A mode the CLI reported that this console does not know, as it was named.
    /// Kept rather than dropped: the CLI gains modes between releases, and the
    /// header must still say something.
    Unknown(String),
}

impl crate::named::Named for Mode {
    fn unknown(name: String) -> Self {
        Mode::Unknown(name)
    }
}

impl Mode {
    /// The mode a CLI name stands for.
    pub fn named(name: &str) -> Mode {
        crate::named::named(name)
    }

    /// The name the CLI takes on its command line and in a mode change.
    pub fn name(&self) -> &str {
        match self {
            Mode::Plan => "plan",
            Mode::Default => "default",
            Mode::DontAsk => "dontAsk",
            Mode::AcceptEdits => "acceptEdits",
            Mode::Auto => "auto",
            Mode::BypassPermissions => "bypassPermissions",
            Mode::Unknown(name) => name,
        }
    }
}

/// Every conversation's last known mode, by session id.
#[derive(Debug)]
pub struct Modes {
    store: PathBuf,
    held: RwLock<BTreeMap<String, Mode>>,
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
    pub fn get(&self, id: &str) -> Option<Mode> {
        self.held.read().get(id).cloned()
    }

    /// Remember a conversation's mode, if it has changed — this runs on every spawn
    /// and every mode request.
    pub fn set(&self, id: &str, mode: &Mode) {
        let all = {
            let mut held = self.held.write();
            if held.get(id) == Some(mode) {
                return;
            }
            held.insert(id.to_string(), mode.clone());
            held.clone()
        };
        self.write(&all);
    }

    /// Written whole each time — one short line per conversation, like
    /// [`crate::gist::Gists`], and a rewrite cannot leave a half-updated entry.
    fn write(&self, all: &BTreeMap<String, Mode>) {
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

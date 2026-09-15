//! What has been written and not sent, per conversation, shared between devices.
//!
//! ⚠ **A draft is a RECORD, not an instruction, and that is what separates this
//! from the queued send memview #90 refused.** A message handed to a session is
//! an instruction, and one delivered minutes later — after the conversation has
//! moved on — is not what was meant. A draft leaves nothing until a person
//! presses send, so it stays true until it is changed and can be replicated
//! without ever acting on its own.
//!
//! **Why the runner holds it.** Both clients talk to this process: the browser
//! on the Mac and the phone over the tunnel. There is no second server to
//! reconcile against, so a draft written on one device is on the other as soon
//! as it is pushed, and the only conflict is a genuine one — both edited while
//! a device was away.
//!
//! **Text only.** A draft can also carry a scaled screenshot, which is hundreds
//! of kilobytes of base64 against a sentence's few hundred bytes; there is no
//! meaningful way to combine two images, and the device that took one is
//! usually the device that wants it. The picture stays in the client's own
//! storage. Reopen that if a picture is ever wanted on the other screen —
//! nothing here assumes it never will be.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// One conversation's unsent words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Draft {
    /// What was being typed.
    pub text: String,
    /// Bumped on every accepted write. A client sends the `rev` it was editing
    /// from, and a mismatch is the conflict — see [`Drafts::put`].
    pub rev: u64,
    /// Unix milliseconds.
    ///
    /// ⚠ **The only thing that distinguishes the two drafts, and deliberately
    /// so.** A field naming the writing device was tried and removed: whoever is
    /// choosing is standing at one of the two, so "the other one" needs no name,
    /// and the useful question is which thought is newer.
    pub at: u64,
}

/// What a write did, which is what the client draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wrote {
    /// Stored. The new state is the caller's own text at the returned revision.
    Stored(Draft),
    /// Refused: somebody else has written since the revision this edit was made
    /// from. Carries THEIRS, because the client cannot resolve without it.
    Conflict(Draft),
}

/// Every conversation's unsent words, by session id.
#[derive(Debug)]
pub struct Drafts {
    store: PathBuf,
    held: RwLock<BTreeMap<String, Draft>>,
}

impl Drafts {
    /// Read what the last run wrote.
    ///
    /// An unreadable file is an empty set and a loud line, not an error, for the
    /// reason [`crate::modes::Modes::load`] gives: a console that will not start
    /// because it cannot remember a preference is worse than one that starts and
    /// asks. Here the cost of the empty case is that a draft is missing from the
    /// OTHER device — the device that typed it still holds its own copy.
    pub fn load(store: PathBuf) -> Self {
        let held = match std::fs::read_to_string(&store) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(held) => held,
                Err(why) => {
                    tracing::error!(
                        "drafts: {} will not parse ({why}) — unsent words will not cross devices",
                        store.display()
                    );
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        Self {
            store,
            held: RwLock::new(held),
        }
    }

    /// This conversation's draft, if any device has written one.
    pub fn get(&self, id: &str) -> Option<Draft> {
        self.held.read().expect("drafts poisoned").get(id).cloned()
    }

    /// Every draft, for the roster — a client that has just connected learns
    /// which conversations hold unsent words without asking per session.
    pub fn all(&self) -> BTreeMap<String, Draft> {
        self.held.read().expect("drafts poisoned").clone()
    }

    /// Record what a device is holding, if it is editing from the current
    /// revision.
    ///
    /// ⚠ **`from` is the revision the edit was MADE against, not the one it
    /// wants.** Equal means nothing changed underneath and the write is taken;
    /// anything else means another device has written since, and the caller is
    /// handed that draft rather than having its own silently dropped or silently
    /// winning.
    ///
    /// ⚠ **A cleared draft is a TOMBSTONE, not a removal, and that is what stops
    /// a sent message coming back.** Sending on the Mac empties its composer,
    /// which arrives here as an empty write. If that erased the entry, the phone
    /// — still holding the words at the old revision — would push them back and
    /// resurrect a message already sent. Keeping the revision means that push
    /// arrives as the conflict it is, with THEIRS empty, and the person decides.
    pub fn put(&self, id: &str, text: &str, from: Option<u64>, at: u64) -> Wrote {
        let (result, all) = {
            let mut held = self.held.write().expect("drafts poisoned");
            let current = held.get(id).cloned();
            if let Some(theirs) = current.as_ref()
                && Some(theirs.rev) != from
            {
                return Wrote::Conflict(theirs.clone());
            }
            let next = Draft {
                text: text.to_string(),
                rev: current.map_or(1, |d| d.rev + 1),
                at,
            };
            held.insert(id.to_string(), next.clone());
            (Wrote::Stored(next), held.clone())
        };
        self.write(&all);
        result
    }

    /// Drop the drafts of conversations that are no longer on disk.
    ///
    /// Same argument as [`crate::gist::Gists::forget`]: a map keyed by id that
    /// is only ever written to grows forever. An empty `alive` is a sweep that
    /// found nothing and is not evidence that everything has gone.
    pub fn forget(&self, alive: &std::collections::BTreeSet<String>) {
        if alive.is_empty() {
            return;
        }
        let all = {
            let mut held = self.held.write().expect("drafts poisoned");
            let before = held.len();
            held.retain(|id, _| alive.contains(id));
            if held.len() == before {
                return;
            }
            tracing::info!(
                "drafts: {} conversation(s) gone from disk, forgetting their unsent words",
                before - held.len()
            );
            held.clone()
        };
        self.write(&all);
    }

    /// Written whole each time — a short line per conversation, like
    /// [`crate::modes::Modes`], and a rewrite cannot leave a half-updated entry.
    fn write(&self, all: &BTreeMap<String, Draft>) {
        if let Ok(text) = serde_json::to_string_pretty(all)
            && let Some(dir) = self.store.parent()
        {
            let _ = std::fs::create_dir_all(dir);
            if let Err(why) = std::fs::write(&self.store, text) {
                tracing::warn!("drafts: could not write {}: {why}", self.store.display());
            }
        }
    }
}
